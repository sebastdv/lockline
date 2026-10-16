# add-check-without-not-valid

**Severity:** error

Flags `ADD CONSTRAINT ... CHECK` without `NOT VALID` on an existing table.

## Why it matters

Adding a `CHECK` constraint scans every row to validate it while an ACCESS EXCLUSIVE lock blocks reads and writes.

## Instead

Add it as `NOT VALID` (instant; new rows are checked) and validate existing rows separately, which only takes a SHARE UPDATE EXCLUSIVE lock.

```sql
ALTER TABLE users ADD CONSTRAINT users_age_check CHECK (age > 0) NOT VALID;
ALTER TABLE users VALIDATE CONSTRAINT users_age_check;
```

## Ignore it when

The table is tiny.
