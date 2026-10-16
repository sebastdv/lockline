# alter-column-type

**Severity:** error

Flags `ALTER COLUMN ... TYPE` that rewrites the table.

## Why it matters

Changing a column's type rewrites the table and all its indexes under an ACCESS EXCLUSIVE lock. The only exceptions lockline recognizes are widenings PostgreSQL applies in place: `varchar(n)` to a longer `varchar`, unbounded `varchar` or `text`; `numeric(p,s)` to a larger precision with the same scale; `varbit(n)` to a longer `varbit`. It can only recognize them when it saw the previous type in the run.

## Instead

Add a new column, write to both, backfill in batches, switch reads, and drop the old one later.

```sql
ALTER TABLE users ADD COLUMN amount_v2 numeric(12,2);
-- deploy code that writes both columns, backfill amount_v2 in batches,
-- switch reads to amount_v2, then drop amount in a later migration.
```

## Ignore it when

The change is a safe widening that lockline could not verify because the table was created outside the analyzed files.
