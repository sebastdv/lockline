# backfill-in-migration

**Severity:** warning

Flags `UPDATE` or `DELETE` without `WHERE` on an existing table.

## Why it matters

Touching every row in one statement holds row locks on all of them until the migration commits, writes a lot of WAL and dead tuples, and can run for a long time inside the migration transaction.

## Instead

Backfill in batches from a separate job, outside the migration transaction.

```sql
UPDATE users SET status = 'active' WHERE id BETWEEN 1 AND 10000 AND status IS NULL;
-- repeat for the next range from a background job
```

## Ignore it when

The table is tiny.
