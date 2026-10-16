# add-unique-without-index

**Severity:** error

Flags `ADD CONSTRAINT ... UNIQUE` or `PRIMARY KEY` that builds its index inside the `ALTER TABLE`.

## Why it matters

The constraint builds a unique index while holding an ACCESS EXCLUSIVE lock, so reads and writes wait for the whole index build.

## Instead

Build the unique index concurrently, then attach it with `USING INDEX` (instant). For a primary key, the columns must already be `NOT NULL`.

```sql
CREATE UNIQUE INDEX CONCURRENTLY users_email_key_idx ON users (email);
ALTER TABLE users ADD CONSTRAINT users_email_key UNIQUE USING INDEX users_email_key_idx;
```

## Ignore it when

The table is tiny.
