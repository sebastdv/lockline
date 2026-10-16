# rewrite-table

**Severity:** error

Flags `VACUUM FULL` and `CLUSTER` on existing tables.

## Why it matters

Both rewrite the entire table and its indexes under an ACCESS EXCLUSIVE lock: nothing can read or write the table until they finish.

## Instead

Use `pg_repack` or `pg_squeeze` to reclaim space online, or a plain `VACUUM`, which does not block reads or writes.

```sql
-- outside migrations, with the pg_repack extension installed:
-- pg_repack --table=users mydb
```

## Ignore it when

You run it in a maintenance window with the application stopped.
