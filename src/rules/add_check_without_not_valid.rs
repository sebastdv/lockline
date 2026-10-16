use pg_query::protobuf::ConstrType;

use super::constraints::{added_constraints, not_valid_then_validate, relation};
use super::{Rule, RuleCtx};
use crate::model::{Finding, Severity};

pub struct AddCheckWithoutNotValid;

impl Rule for AddCheckWithoutNotValid {
    fn id(&self) -> &'static str {
        "add-check-without-not-valid"
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        let target = ctx.existing_target()?;
        let added = added_constraints(&ctx.stmt.node)
            .into_iter()
            .find(|a| a.is(ConstrType::ConstrCheck) && !a.constraint.skip_validation)?;
        Some(self.finding(
            format!(
                "Adding a CHECK constraint scans all of \"{}\" under an ACCESS EXCLUSIVE lock.",
                target.table.plain()
            ),
            Some(not_valid_then_validate(relation(&ctx.stmt.node)?, &added)),
        ))
    }
}
