use super::{Rule, RuleCtx};
use crate::catalog::forbidden_in_transaction;
use crate::model::{Finding, Severity};

pub struct CannotRunInTransaction;

impl Rule for CannotRunInTransaction {
    fn id(&self) -> &'static str {
        "cannot-run-in-transaction"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        if !ctx.in_transaction {
            return None;
        }
        let label = forbidden_in_transaction(&ctx.stmt.node)?;
        Some(self.finding(
            format!("{label} cannot run inside a transaction block; this migration will fail."),
            Some(
                "Run it outside a transaction. In Laravel, add \"public $withinTransaction = false;\" to the migration class."
                    .to_string(),
            ),
        ))
    }
}
