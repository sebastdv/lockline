use pg_query::NodeEnum;

use super::{Rule, RuleCtx, quoted_names};
use crate::model::{Cost, Finding, Severity};

pub struct RewriteTable;

impl Rule for RewriteTable {
    fn id(&self) -> &'static str {
        "rewrite-table"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let label = match &ctx.stmt.node {
            NodeEnum::VacuumStmt(_) => "VACUUM FULL",
            NodeEnum::ClusterStmt(_) => "CLUSTER",
            _ => return None,
        };
        let targets: Vec<_> = ctx
            .existing_targets()
            .into_iter()
            .filter(|e| e.cost == Cost::Rewrite)
            .collect();
        if targets.is_empty() {
            return None;
        }
        Some(self.finding(
            format!(
                "{label} rewrites {} under an ACCESS EXCLUSIVE lock; nothing can read or write it until it finishes.",
                quoted_names(&targets)
            ),
            Some(
                "Use pg_repack or pg_squeeze to rebuild the table online, outside of migrations."
                    .to_string(),
            ),
        ))
    }
}
