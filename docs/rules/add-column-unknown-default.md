# add-column-unknown-default

**Severity:** warning

Flags `ADD COLUMN ... DEFAULT f()` where lockline cannot tell whether `f` is volatile.

## Why it matters

If the function is volatile, PostgreSQL rewrites the whole table under an ACCESS EXCLUSIVE lock (see `add-column-rewrite`); if it is stable or immutable, the change is instant. lockline only knows the volatility of built-in functions.

## Instead

Check the function's volatility and, if it is not volatile, suppress the warning with the reason.

```sql
SELECT proname, provolatile FROM pg_proc WHERE proname = 'make_code';
-- 'v' = volatile (rewrite), 's' = stable, 'i' = immutable (fast)
```

## Ignore it when

You checked `pg_proc.provolatile` and the function is stable or immutable: add `-- lockline:ignore add-column-unknown-default <function> is immutable`.
