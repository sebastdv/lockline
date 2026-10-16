# ignore-without-reason

**Severity:** warning

Flags a `lockline:ignore` comment that does not say why the finding is safe to ignore.

## Why it matters

A suppression without a reason hides a risk without telling the next reader why it was accepted. The finding is still suppressed.

## Instead

Write the reason after the rule ids.

```sql
-- lockline:ignore create-index-not-concurrently the table has under 1000 rows
CREATE INDEX idx_flags_name ON flags (name);
```

## Ignore it when

Never; write the reason.
