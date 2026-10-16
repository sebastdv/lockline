use pg_query::NodeEnum;
use pg_query::protobuf::{AlterTableType, ObjectType};

use super::{Rule, RuleCtx, table_alter_cmds};
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
        let target = ctx.existing_target()?;
        let name = match &ctx.stmt.node {
            NodeEnum::DropStmt(stmt) if stmt.remove_type == ObjectType::ObjectTable as i32 => {
                target.table.to_string()
            }
            node => table_alter_cmds(node)
                .into_iter()
                .find(|cmd| cmd.subtype == AlterTableType::AtDropColumn as i32)?
                .name
                .clone(),
        };
        Some(self.finding(
            format!(
                "Dropping \"{name}\" breaks application code that still uses it while the deploy rolls out."
            ),
            Some(
                "Stop using it in the code first, deploy, then drop it in a later migration."
                    .to_string(),
            ),
        ))
    }
}
