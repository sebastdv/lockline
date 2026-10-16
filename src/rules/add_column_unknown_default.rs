use pg_query::protobuf::ConstrType;

use super::{Rule, RuleCtx, added_column, column_constraints, table_alter_cmds};
use crate::model::{Finding, Severity};
use crate::volatility::{Volatility, classify_default, unknown_call};

pub struct AddColumnUnknownDefault;

impl Rule for AddColumnUnknownDefault {
    fn id(&self) -> &'static str {
        "add-column-unknown-default"
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        let expr = table_alter_cmds(&ctx.stmt.node)
            .into_iter()
            .filter_map(added_column)
            .flat_map(|column| {
                column_constraints(column)
                    .filter(|c| c.contype == ConstrType::ConstrDefault as i32)
                    .filter_map(|c| c.raw_expr.as_deref())
                    .chain(column.raw_default.as_deref())
            })
            .find(|expr| classify_default(expr) == Volatility::Unknown)?;
        let name = unknown_call(expr).unwrap_or_else(|| "expression".to_string());
        Some(self.finding(
            format!(
                "lockline cannot tell whether the default {name} is volatile; if it is, \"{}\" is rewritten.",
                target.table.plain()
            ),
            Some(
                "Use a constant default, or check the function's volatility (pg_proc.provolatile) and add a lockline:ignore comment with the reason."
                    .to_string(),
            ),
        ))
    }
}
