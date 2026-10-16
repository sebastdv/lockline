use pg_query::protobuf::ConstrType;

use super::constraints::{added_constraints, not_valid_then_validate, relation};
use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity, TableRef};

pub struct AddForeignKeyWithoutNotValid;

impl Rule for AddForeignKeyWithoutNotValid {
    fn id(&self) -> &'static str {
        "add-foreign-key-without-not-valid"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        let added = added_constraints(&ctx.stmt.node)
            .into_iter()
            .find(|a| a.is(ConstrType::ConstrForeign) && !a.constraint.skip_validation)?;
        let referenced = TableRef::from_range_var(added.constraint.pktable.as_ref()?);
        let table = target.table.plain();
        let referenced = referenced.plain();
        Some(self.finding(
            format!(
                "Adding a foreign key scans \"{table}\" and blocks writes to \"{table}\" and \"{referenced}\" until it is validated."
            ),
            Some(not_valid_then_validate(relation(&ctx.stmt.node)?, &added)),
        ))
    }
}
