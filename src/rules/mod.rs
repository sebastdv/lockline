//! Lint rules. Each rule looks at one statement in context and may report one finding.

mod add_column_rewrite;
mod cannot_run_in_transaction;
mod create_index_not_concurrently;

use crate::model::{Analysis, Finding, Severity};
use crate::parse::Statement;

pub struct RuleCtx<'a> {
    pub stmt: &'a Statement,
    pub analysis: &'a Analysis,
    /// Every table the statement locks was created earlier in the same unit.
    pub new_table: bool,
    pub in_transaction: bool,
}

pub trait Rule {
    fn id(&self) -> &'static str;
    fn severity(&self) -> Severity;
    fn check(&self, ctx: &RuleCtx) -> Option<Finding>;

    fn finding(&self, message: String, help: Option<String>) -> Finding {
        Finding {
            rule: self.id(),
            severity: self.severity(),
            message,
            help,
            line: None,
        }
    }
}

pub fn all() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(create_index_not_concurrently::CreateIndexNotConcurrently),
        Box::new(cannot_run_in_transaction::CannotRunInTransaction),
        Box::new(add_column_rewrite::AddColumnRewrite),
    ]
}
