# refresh-matview-not-concurrently

**Severity:** warning

Flags `REFRESH MATERIALIZED VIEW` without `CONCURRENTLY`.

## Why it matters

A plain refresh takes an ACCESS EXCLUSIVE lock on the view, so every query that reads it waits until the refresh finishes.

## Instead

Refresh concurrently. It needs a unique index on the view and takes an EXCLUSIVE lock, which still allows reads.

```sql
CREATE UNIQUE INDEX CONCURRENTLY stats_day_idx ON stats (day);
REFRESH MATERIALIZED VIEW CONCURRENTLY stats;
```

## Ignore it when

Nothing reads the view yet.
