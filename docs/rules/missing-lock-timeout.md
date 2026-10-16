# missing-lock-timeout

**Severity:** warning

Flags a migration that takes an ACCESS EXCLUSIVE lock without setting `lock_timeout` first. Reported once per file.

## Why it matters

An ACCESS EXCLUSIVE lock has to wait for every running query on the table. While it waits, every new query on that table queues behind it, so one slow report can turn a fast `ALTER TABLE` into an outage. A `lock_timeout` makes the migration fail fast instead, so you can retry it.

## Instead

Set a short `lock_timeout` at the start of the migration and retry the migration when it times out.

```sql
SET lock_timeout = '5s';  -- or SET LOCAL lock_timeout inside a transaction
ALTER TABLE users ADD COLUMN nickname text;
```

## Ignore it when

Your migration runner sets `lock_timeout` for every session, or the table is not used by the application yet.
