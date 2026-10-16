use pg_query::NodeEnum;
use pg_query::protobuf::{DefElem, DefElemAction, Node, ReindexObjectType};

use super::{Rule, RuleCtx};
use crate::catalog::forbidden_in_transaction;
use crate::model::{Finding, Severity};

pub struct ReindexNotConcurrently;

impl Rule for ReindexNotConcurrently {
    fn id(&self) -> &'static str {
        "reindex-not-concurrently"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let NodeEnum::ReindexStmt(stmt) = &ctx.stmt.node else {
            return None;
        };
        let targets_relation = [
            ReindexObjectType::ReindexObjectTable,
            ReindexObjectType::ReindexObjectIndex,
        ]
        .iter()
        .any(|k| *k as i32 == stmt.kind);
        if !targets_relation || forbidden_in_transaction(&ctx.stmt.node).is_some() {
            return None;
        }
        let target = ctx.existing_target()?;
        let mut concurrent = stmt.clone();
        concurrent.params.retain(|p| {
            !matches!(p.node.as_ref(), Some(NodeEnum::DefElem(d)) if d.defname == "concurrently")
        });
        concurrent.params.push(Node {
            node: Some(NodeEnum::DefElem(Box::new(DefElem {
                defname: "concurrently".to_string(),
                // The C deparser asserts on enum value 0 (Undefined).
                defaction: DefElemAction::DefelemUnspec as i32,
                location: -1,
                ..DefElem::default()
            }))),
        });
        let help = NodeEnum::ReindexStmt(concurrent)
            .deparse()
            .unwrap_or_else(|_| format!("{} -- add CONCURRENTLY", ctx.stmt.text));
        Some(self.finding(
            format!(
                "REINDEX without CONCURRENTLY blocks writes to \"{}\" while the indexes are rebuilt.",
                target.table
            ),
            Some(format!(
                "{help};\n(CONCURRENTLY cannot run inside a transaction block)"
            )),
        ))
    }
}
