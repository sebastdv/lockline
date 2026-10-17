# ignore-unknown-rule

**Severity:** warning

Flags a `lockline:ignore` comment that names a rule lockline does not have.

## Why it matters

A misspelled rule id suppresses nothing, so the comment looks like an accepted risk while the finding it meant to silence is still reported, or worse, the author thinks a different rule is covered.

## Instead

Use the exact id from `lockline rules`.

```sql
-- lockline:ignore drop-object no code reads this table since release 4.2
DROP TABLE legacy_sessions;
```

## Ignore it when

Never; fix the rule id.
