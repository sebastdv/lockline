use pg_query::protobuf::ConstrType;

use super::constraints::added_constraints;
use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity};

pub struct AddUniqueWithoutIndex;

impl Rule for AddUniqueWithoutIndex {
    fn id(&self) -> &'static str {
        "add-unique-without-index"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        let table = &target.table;
        let added = added_constraints(&ctx.stmt.node).into_iter().find(|a| {
            (a.is(ConstrType::ConstrUnique) || a.is(ConstrType::ConstrPrimary))
                && a.constraint.indexname.is_empty()
        })?;
        let kind = if added.is(ConstrType::ConstrPrimary) {
            "PRIMARY KEY"
        } else {
            "UNIQUE"
        };
        let name = added.name(table);
        let index = format!("{name}_idx");
        let columns = added.columns().join(", ");
        Some(self.finding(
            format!(
                "Adding a {kind} constraint builds an index under an ACCESS EXCLUSIVE lock on \"{table}\"."
            ),
            Some(format!(
                "CREATE UNIQUE INDEX CONCURRENTLY {index} ON {table} ({columns});\n\
                 ALTER TABLE {table} ADD CONSTRAINT {name} {kind} USING INDEX {index};"
            )),
        ))
    }
}
