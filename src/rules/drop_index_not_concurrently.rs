use pg_query::NodeEnum;
use pg_query::protobuf::{DropBehavior, ObjectType};

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
        // CONCURRENTLY takes one index at a time and does not support CASCADE.
        let mut lines: Vec<String> = stmt
            .objects
            .iter()
            .map(|object| {
                let mut single = stmt.clone();
                single.objects = vec![object.clone()];
                single.concurrent = true;
                single.behavior = DropBehavior::DropRestrict as i32;
                NodeEnum::DropStmt(single)
                    .deparse()
                    .map(|sql| format!("{sql};"))
                    .unwrap_or_else(|_| {
                        format!(
                            "{};",
                            ctx.stmt.text.replacen("INDEX ", "INDEX CONCURRENTLY ", 1)
                        )
                    })
            })
            .collect();
        if stmt.behavior == DropBehavior::DropCascade as i32 {
            lines.push(
                "(CONCURRENTLY does not support CASCADE; drop the dependent objects first.)"
                    .to_string(),
            );
        }
        lines.push("(CONCURRENTLY cannot run inside a transaction block)".to_string());
        let help = lines.join("\n");
        Some(self.finding(
            "Dropping an index without CONCURRENTLY takes an ACCESS EXCLUSIVE lock on its table."
                .to_string(),
            Some(help),
        ))
    }
}
