use pg_query::NodeEnum;
use pg_query::protobuf::AlterTableType;

use super::{Rule, RuleCtx, table_alter_cmds};
use crate::model::{Finding, Severity};
use crate::schema::{TypeSpec, is_safe_widening};

pub struct AlterColumnType;

const REWRITE_HELP: &str =
    "add a new column, backfill it in batches, and switch reads and writes over.";

impl Rule for AlterColumnType {
    fn id(&self) -> &'static str {
        "alter-column-type"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        for cmd in table_alter_cmds(&ctx.stmt.node) {
            if cmd.subtype != AlterTableType::AtAlterColumnType as i32 {
                continue;
            }
            let Some(NodeEnum::ColumnDef(column)) = cmd.def.as_ref().and_then(|d| d.node.as_ref())
            else {
                continue;
            };
            let previous = ctx.schema.column_type(&target.table, &cmd.name);
            let new_type = column.type_name.as_ref().map(TypeSpec::from_type_name);
            let safe = column.raw_default.is_none()
                && matches!((previous, &new_type), (Some(old), Some(new)) if is_safe_widening(old, new));
            if safe {
                continue;
            }
            let help = if previous.is_some() {
                format!("A{}", &REWRITE_HELP[1..])
            } else {
                format!(
                    "If this only widens varchar(n) or numeric precision it does not rewrite; lockline did not see the previous type.\nOtherwise {REWRITE_HELP}"
                )
            };
            return Some(self.finding(
                format!(
                    "Changing the type of \"{}\" rewrites \"{}\" and its indexes under an ACCESS EXCLUSIVE lock.",
                    cmd.name, target.table.plain()
                ),
                Some(help),
            ));
        }
        None
    }
}
