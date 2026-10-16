use super::{Rule, RuleCtx, added_column, table_alter_cmds};
use crate::catalog::add_column_cost;
use crate::model::{Cost, Finding, Severity};

pub struct AddColumnRewrite;

impl Rule for AddColumnRewrite {
    fn id(&self) -> &'static str {
        "add-column-rewrite"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let effect = ctx.existing_target()?;
        // Only the ADD COLUMN itself counts: ALTER COLUMN TYPE has its own rule.
        table_alter_cmds(&ctx.stmt.node)
            .into_iter()
            .filter_map(added_column)
            .find(|column| add_column_cost(column) == Cost::Rewrite)?;
        Some(self.finding(
            format!(
                "Adding this column rewrites \"{}\" and all its indexes while holding an ACCESS EXCLUSIVE lock.",
                effect.table.plain()
            ),
            Some(
                "Add the column without a default (or with a constant default), then backfill in batches and set the default afterwards."
                    .to_string(),
            ),
        ))
    }
}
