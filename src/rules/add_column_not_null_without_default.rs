use pg_query::protobuf::ConstrType;

use super::{Rule, RuleCtx, added_column, column_constraints, table_alter_cmds};
use crate::model::{Finding, Severity};
use crate::volatility::last_name;

pub struct AddColumnNotNullWithoutDefault;

/// Column types and constraints that fill every existing row with a value.
const SERIAL_TYPES: &[&str] = &[
    "serial",
    "bigserial",
    "smallserial",
    "serial2",
    "serial4",
    "serial8",
];

impl Rule for AddColumnNotNullWithoutDefault {
    fn id(&self) -> &'static str {
        "add-column-not-null-without-default"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        let column = table_alter_cmds(&ctx.stmt.node)
            .into_iter()
            .filter_map(added_column)
            .find(|column| {
                let types: Vec<i32> = column_constraints(column).map(|c| c.contype).collect();
                let has = |t: ConstrType| types.contains(&(t as i32));
                let type_name = column
                    .type_name
                    .as_ref()
                    .map_or("", |t| last_name(&t.names));
                has(ConstrType::ConstrNotnull)
                    && column.raw_default.is_none()
                    && !has(ConstrType::ConstrDefault)
                    && !has(ConstrType::ConstrIdentity)
                    && !has(ConstrType::ConstrGenerated)
                    && !SERIAL_TYPES.contains(&type_name)
            });
        column?;
        Some(
            self.finding(
                format!(
                    "Adding a NOT NULL column without a DEFAULT fails if \"{}\" has any rows.",
                    target.table
                ),
                Some(
                    "Add a DEFAULT, or add the column as nullable, backfill it, then SET NOT NULL."
                        .to_string(),
                ),
            ),
        )
    }
}
