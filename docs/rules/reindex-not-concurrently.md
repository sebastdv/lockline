# reindex-not-concurrently

**Severity:** error

Flags `REINDEX TABLE` or `REINDEX INDEX` without `CONCURRENTLY`.

## Why it matters

`REINDEX` takes a SHARE lock on the table, blocking writes, plus an ACCESS EXCLUSIVE lock on each index being rebuilt, blocking the queries that use it. Rebuilding takes as long as building the index from scratch.

## Instead

Use `REINDEX ... CONCURRENTLY` (PostgreSQL 12+), outside a transaction block.

```sql
REINDEX TABLE CONCURRENTLY users;
```

## Ignore it when

You are rebuilding indexes during a maintenance window with traffic stopped.
