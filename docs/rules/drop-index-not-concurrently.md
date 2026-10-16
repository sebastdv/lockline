# drop-index-not-concurrently

**Severity:** warning

Flags `DROP INDEX` without `CONCURRENTLY`.

## Why it matters

`DROP INDEX` takes an ACCESS EXCLUSIVE lock on the index's table. The drop itself is fast, but the lock has to wait for every running query on the table, and every new query queues behind it while it waits.

## Instead

Drop it concurrently (outside a transaction block).

```sql
DROP INDEX CONCURRENTLY idx_users_email;
```

## Ignore it when

The table is small and the migration sets a `lock_timeout`.
