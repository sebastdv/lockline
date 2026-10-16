# add-foreign-key-without-not-valid

**Severity:** error

Flags a new foreign key without `NOT VALID` on an existing table.

## Why it matters

Adding a foreign key takes a SHARE ROW EXCLUSIVE lock on both tables and scans the referencing table to validate every row. Writes to both tables wait until the scan finishes.

## Instead

Add it as `NOT VALID` and validate it in a second step, which does not block writes.

```sql
ALTER TABLE orders ADD CONSTRAINT orders_user_id_fkey FOREIGN KEY (user_id) REFERENCES users (id) NOT VALID;
ALTER TABLE orders VALIDATE CONSTRAINT orders_user_id_fkey;
```

## Ignore it when

The referencing table is tiny.
