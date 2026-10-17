use super::{Rule, RuleCtx, added_column, table_alter_cmds};
use crate::catalog::{RewriteCause, rewrite_cause};
use crate::model::{Finding, Severity};

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
        let cause = table_alter_cmds(&ctx.stmt.node)
            .into_iter()
            .filter_map(added_column)
            .find_map(rewrite_cause)?;
        Some(self.finding(
            format!(
                "Adding this column rewrites \"{}\" and all its indexes while holding an ACCESS EXCLUSIVE lock.",
                effect.table.plain()
            ),
            Some(help(cause).to_string()),
        ))
    }
}

fn help(cause: RewriteCause) -> &'static str {
    match cause {
        RewriteCause::VolatileDefault => {
            "Add the column without a default (or with a constant default), then backfill in batches and set the default afterwards."
        }
        RewriteCause::Serial | RewriteCause::Identity => {
            "Add a plain nullable column, backfill it in batches from a sequence, make it NOT NULL with the set-not-null pattern, then ALTER COLUMN ... ADD GENERATED ... AS IDENTITY (catalog-only)."
        }
        RewriteCause::Generated => {
            "PostgreSQL cannot add a stored generated column without rewriting. Add a regular column kept in sync by the application or a trigger, and backfill it in batches."
        }
    }
}
