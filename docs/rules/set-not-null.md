# set-not-null

**Severity:** error

Flags `ALTER COLUMN ... SET NOT NULL` on an existing table.

## Why it matters

PostgreSQL scans the whole table to prove there are no `NULL`s, holding an ACCESS EXCLUSIVE lock that blocks reads and writes. Since PostgreSQL 12 the scan is skipped when a validated `CHECK (column IS NOT NULL)` constraint already proves it.

There is no scan either when the column already is `NOT NULL`, as with Laravel's `->change()`, which repeats `SET NOT NULL` for every non-nullable column. lockline does not flag that case when it saw the column become `NOT NULL` (declared so, part of the primary key, or set earlier) in the analyzed files or in `--history`.

## Instead

Add the check as `NOT VALID`, validate it (this only takes a SHARE UPDATE EXCLUSIVE lock), then set `NOT NULL` and drop the check.

```sql
ALTER TABLE users ADD CONSTRAINT users_email_not_null CHECK (email IS NOT NULL) NOT VALID;
ALTER TABLE users VALIDATE CONSTRAINT users_email_not_null;
ALTER TABLE users ALTER COLUMN email SET NOT NULL;
ALTER TABLE users DROP CONSTRAINT users_email_not_null;
```

## Ignore it when

The validated check exists, or the column already is `NOT NULL`, but that happened outside the analyzed files and `--history`.
