use super::{Rule, RuleCtx};
use crate::locks::LockMode;
use crate::model::{Analysis, Finding, Severity};

pub struct MissingLockTimeout;

impl Rule for MissingLockTimeout {
    fn id(&self) -> &'static str {
        "missing-lock-timeout"
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn once_per_unit(&self) -> bool {
        true
    }

    fn check(&self, ctx: &RuleCtx) -> Option<Finding> {
        if ctx.lock_timeout_set {
            return None;
        }
        let Analysis::Locks(effects) = ctx.analysis else {
            return None;
        };
        let effect = effects
            .iter()
            .find(|e| e.lock == LockMode::AccessExclusive && !e.new_table)?;
        Some(self.finding(
            format!(
                "This migration takes an ACCESS EXCLUSIVE lock on \"{}\" without a lock_timeout; if the lock has to wait, every query on the table queues behind it.",
                effect.table.plain()
            ),
            Some(
                "Add SET lock_timeout = '5s'; at the start of the migration (SET LOCAL inside a transaction) and retry on failure."
                    .to_string(),
            ),
        ))
    }
}
