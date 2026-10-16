use pg_query::NodeEnum;
use pg_query::protobuf::ObjectType;

use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity};

pub struct RenameObject;

impl Rule for RenameObject {
    fn id(&self) -> &'static str {
        "rename-object"
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let NodeEnum::RenameStmt(stmt) = &ctx.stmt.node else {
            return None;
        };
        let target = ctx.existing_target()?;
        let name = if stmt.rename_type == ObjectType::ObjectTable as i32 {
            target.table.to_string()
        } else if stmt.rename_type == ObjectType::ObjectColumn as i32 {
            stmt.subname.clone()
        } else {
            return None;
        };
        Some(self.finding(
            format!(
                "Renaming \"{name}\" breaks application code that still uses the old name while the deploy rolls out."
            ),
            Some(
                "Use expand/contract: add the new name, deploy code that uses it, then remove the old one in a later migration."
                    .to_string(),
            ),
        ))
    }
}
