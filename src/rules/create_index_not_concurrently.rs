use pg_query::NodeEnum;
use pg_query::protobuf::IndexStmt;

use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity, TableRef};

pub struct CreateIndexNotConcurrently;

impl Rule for CreateIndexNotConcurrently {
    fn id(&self) -> &'static str {
        "create-index-not-concurrently"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let NodeEnum::IndexStmt(stmt) = &ctx.stmt.node else {
            return None;
        };
        if stmt.concurrent || ctx.new_table {
            return None;
        }
        let table = TableRef::from_range_var(stmt.relation.as_ref()?);
        Some(self.finding(
            format!(
                "Building an index without CONCURRENTLY blocks all writes to \"{table}\" until the build finishes."
            ),
            Some(format!(
                "{};\n(CONCURRENTLY cannot run inside a transaction block)",
                concurrent_version(stmt, &ctx.stmt.text)
            )),
        ))
    }
}

/// The same CREATE INDEX with CONCURRENTLY, deparsed from the AST when possible.
fn concurrent_version(stmt: &IndexStmt, original: &str) -> String {
    let mut concurrent = stmt.clone();
    concurrent.concurrent = true;
    NodeEnum::IndexStmt(Box::new(concurrent))
        .deparse()
        .unwrap_or_else(|_| original.replacen("INDEX ", "INDEX CONCURRENTLY ", 1))
}
