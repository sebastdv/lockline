# drop-object

**Severity:** warning

Flags dropping a table or a column that already exists.

## Why it matters

Servers still running the previous version of the code keep querying the dropped object until the deploy finishes, and those queries fail. Dropping is also irreversible.

## Instead

Stop using it in the code first, deploy, then drop it in a later migration.

```sql
-- 1. remove every use of users.legacy from the code and deploy
-- 2. in a later migration:
ALTER TABLE users DROP COLUMN legacy;
```

## Ignore it when

The code already stopped using it in an earlier deploy: add `-- lockline:ignore drop-object no code reads it since <release>`.
