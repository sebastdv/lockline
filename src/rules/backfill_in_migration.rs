use pg_query::NodeEnum;

use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity};

pub struct BackfillInMigration;

impl Rule for BackfillInMigration {
    fn id(&self) -> &'static str {
        "backfill-in-migration"
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let verb = match &ctx.stmt.node {
            NodeEnum::UpdateStmt(stmt) if stmt.where_clause.is_none() => "Updating",
            NodeEnum::DeleteStmt(stmt) if stmt.where_clause.is_none() => "Deleting",
            _ => return None,
        };
        let table = &ctx.existing_target()?.table;
        Some(self.finding(
            format!(
                "{verb} every row of \"{table}\" in a migration holds row locks on the whole table and can run for a long time."
            ),
            Some(
                "Backfill in batches (for example by id ranges) from a separate job, outside the migration transaction."
                    .to_string(),
            ),
        ))
    }
}
