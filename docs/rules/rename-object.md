# rename-object

**Severity:** warning

Flags renaming a table or a column that already exists.

## Why it matters

During a deploy, servers running the previous version of the code still use the old name. Between the migration and the last old server going away, those queries fail.

## Instead

Use expand/contract: add the new name, move the code over, and remove the old name in a later migration.

```sql
ALTER TABLE users ADD COLUMN mail text;
-- deploy code that writes both columns, backfill mail, switch reads,
-- then drop email in a later migration.
```

## Ignore it when

Nothing uses the table or column yet, or you deploy with downtime.
