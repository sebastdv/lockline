# add-column-not-null-without-default

**Severity:** error

Flags `ADD COLUMN ... NOT NULL` without a `DEFAULT` on an existing table.

## Why it matters

Every existing row would get `NULL` in the new column, so PostgreSQL rejects the statement with `column contains null values` as soon as the table has one row. The migration works on an empty development database and fails in production.

## Instead

Give the column a constant default (fast since PostgreSQL 11), or add it as nullable, backfill it, and then apply the `set-not-null` pattern.

```sql
ALTER TABLE users ADD COLUMN status text NOT NULL DEFAULT 'active';
```

## Ignore it when

The table is guaranteed to be empty when the migration runs.
