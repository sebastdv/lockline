use pg_query::NodeEnum;
use pg_query::protobuf::ObjectType;

use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity};

pub struct DropIndexNotConcurrently;

impl Rule for DropIndexNotConcurrently {
    fn id(&self) -> &'static str {
        "drop-index-not-concurrently"
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let NodeEnum::DropStmt(stmt) = &ctx.stmt.node else {
            return None;
        };
        if stmt.remove_type != ObjectType::ObjectIndex as i32 || stmt.concurrent {
            return None;
        }
        ctx.existing_target()?;
        let mut concurrent = stmt.clone();
        concurrent.concurrent = true;
        let help = NodeEnum::DropStmt(concurrent)
            .deparse()
            .unwrap_or_else(|_| ctx.stmt.text.replacen("INDEX ", "INDEX CONCURRENTLY ", 1));
        Some(self.finding(
            "Dropping an index without CONCURRENTLY takes an ACCESS EXCLUSIVE lock on its table."
                .to_string(),
            Some(format!(
                "{help};\n(CONCURRENTLY cannot run inside a transaction block)"
            )),
        ))
    }
}
