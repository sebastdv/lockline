use pg_query::NodeEnum;
use pg_query::protobuf::{AlterTableType, ObjectType};

use super::{Rule, RuleCtx, quoted_names, table_alter_cmds};
use crate::model::{Finding, Severity};

pub struct DropObject;

impl Rule for DropObject {
    fn id(&self) -> &'static str {
        "drop-object"
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let names = match &ctx.stmt.node {
            NodeEnum::DropStmt(stmt) if stmt.remove_type == ObjectType::ObjectTable as i32 => {
                let targets = ctx.existing_targets();
                if targets.is_empty() {
                    return None;
                }
                quoted_names(&targets)
            }
            node => {
                ctx.existing_target()?;
                let column = table_alter_cmds(node)
                    .into_iter()
                    .find(|cmd| cmd.subtype == AlterTableType::AtDropColumn as i32)?;
                format!("\"{}\"", column.name)
            }
        };
        Some(self.finding(
            format!(
                "Dropping {names} breaks application code that still uses it while the deploy rolls out."
            ),
            Some(
                "Stop using it in the code first, deploy, then drop it in a later migration."
                    .to_string(),
            ),
        ))
    }
}
