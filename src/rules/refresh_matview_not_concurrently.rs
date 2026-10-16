use pg_query::NodeEnum;

use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity};

pub struct RefreshMatviewNotConcurrently;

impl Rule for RefreshMatviewNotConcurrently {
    fn id(&self) -> &'static str {
        "refresh-matview-not-concurrently"
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let NodeEnum::RefreshMatViewStmt(stmt) = &ctx.stmt.node else {
            return None;
        };
        if stmt.concurrent {
            return None;
        }
        let view = &ctx.existing_target()?.table;
        Some(self.finding(
            format!("Refreshing without CONCURRENTLY blocks every read of \"{view}\" until it finishes."),
            Some(format!(
                "REFRESH MATERIALIZED VIEW CONCURRENTLY {view};\n(requires a unique index on the view)"
            )),
        ))
    }
}
