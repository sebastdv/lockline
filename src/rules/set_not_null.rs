use pg_query::protobuf::AlterTableType;

use super::{Rule, RuleCtx, table_alter_cmds};
use crate::model::{Finding, Severity, quote_ident};

pub struct SetNotNull;

impl Rule for SetNotNull {
    fn id(&self) -> &'static str {
        "set-not-null"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        let table = &target.table;
        let cmd = table_alter_cmds(&ctx.stmt.node).into_iter().find(|cmd| {
            cmd.subtype == AlterTableType::AtSetNotNull as i32
                && !ctx.schema.has_validated_not_null_check(table, &cmd.name)
        })?;
        let column = quote_ident(&cmd.name);
        let check = quote_ident(&format!("{}_{}_not_null", table.name, cmd.name));
        Some(self.finding(
            format!(
                "SET NOT NULL scans all of \"{}\" under an ACCESS EXCLUSIVE lock to check for NULLs.",
                table.plain()
            ),
            Some(format!(
                "ALTER TABLE {table} ADD CONSTRAINT {check} CHECK ({column} IS NOT NULL) NOT VALID;\n\
                 ALTER TABLE {table} VALIDATE CONSTRAINT {check};\n\
                 ALTER TABLE {table} ALTER COLUMN {column} SET NOT NULL;\n\
                 ALTER TABLE {table} DROP CONSTRAINT {check};"
            )),
        ))
    }
}
