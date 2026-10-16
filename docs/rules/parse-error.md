# parse-error

**Severity:** error

The file is not valid PostgreSQL 17 SQL, so lockline cannot analyze any of it.

## Why it matters

A file lockline cannot read is a file whose locks nobody checked. Common causes are `psql` meta-commands (`\c`, `\i`), template placeholders such as `${schema}`, and SQL written for another database.

## Instead

Fix the syntax, render templates first, or exclude the file in `lockline.toml`.

```sql
-- render templates before running lockline, or exclude the file:
-- lockline.toml
-- exclude = ["migrations/legacy/**"]
```

## Ignore it when

It cannot be suppressed with a comment; exclude the file instead.
