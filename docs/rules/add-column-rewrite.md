# add-column-rewrite

**Severity:** error

Flags `ADD COLUMN` that forces PostgreSQL to write a value into every existing row.

## Why it matters

Since PostgreSQL 11, a column with no default or a non-volatile default is added by changing the catalog only. A volatile default (`random()`, `gen_random_uuid()`, `clock_timestamp()`), `serial`, an identity column or a stored generated column needs a value computed per row, so the whole table and its indexes are rewritten while an ACCESS EXCLUSIVE lock blocks reads and writes.

## Instead

Add the column without the default, set the default for new rows, and backfill the old rows in batches.

```sql
ALTER TABLE users ADD COLUMN api_token uuid;
ALTER TABLE users ALTER COLUMN api_token SET DEFAULT gen_random_uuid();
-- then backfill existing rows in batches, outside the migration:
UPDATE users SET api_token = gen_random_uuid() WHERE id BETWEEN 1 AND 10000 AND api_token IS NULL;
```

## Ignore it when

The table was empty or tiny when the migration runs.
