# cannot-run-in-transaction

**Severity:** error

Flags statements PostgreSQL refuses to run inside a transaction block: `CREATE`/`DROP INDEX CONCURRENTLY`, `REINDEX CONCURRENTLY`, `DETACH PARTITION CONCURRENTLY` and `VACUUM`.

## Why it matters

Most migration tools run each migration inside a transaction (Laravel on PostgreSQL, Rails, Django, Flyway). These statements then fail with `cannot run inside a transaction block` and the deploy stops.

## Instead

Mark the migration as non-transactional in your tool, and keep the statement alone in its own migration.

```sql
-- Laravel:  public $withinTransaction = false;
-- Rails:    disable_ddl_transaction!
-- Django:   atomic = False
-- Flyway:   -- flyway:executeInTransaction=false
```

## Ignore it when

Your tool runs this file without a transaction: pass `--transaction none` or set `transaction = "none"` in `lockline.toml` instead of ignoring it.
