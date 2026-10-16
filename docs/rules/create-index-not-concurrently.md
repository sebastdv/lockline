# create-index-not-concurrently

**Severity:** error

Flags `CREATE INDEX` without `CONCURRENTLY` on a table that already exists.

## Why it matters

A plain `CREATE INDEX` takes a SHARE lock on the table for the whole build. Reads keep working, but every `INSERT`, `UPDATE` and `DELETE` waits until the index is finished, and the build time grows with the size of the table.

## Instead

Build the index concurrently. It takes longer and cannot run inside a transaction block; if it fails it leaves an `INVALID` index that you drop and build again.

```sql
CREATE INDEX CONCURRENTLY idx_users_email ON users (email);
```

## Ignore it when

The table is tiny or not used by the application yet.
