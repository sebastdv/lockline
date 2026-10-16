# lockline

**Know what your PostgreSQL migration locks before it hits production.**

A migration that takes 20 ms on your laptop can block every write to a 50M-row table for
minutes in production. `lockline` reads your migration SQL with the real PostgreSQL parser
and tells you, for every statement, which lock it takes, what that lock blocks, and whether
it rewrites or scans the table. When a pattern is dangerous, it explains why and suggests
the safe version.

> **Status:** early development (milestone 2 of 4). The lock catalog covers `ALTER TABLE`,
> indexes, `DROP`, `REINDEX`, `VACUUM`, `CLUSTER`, `REFRESH`, triggers, DML, `COMMENT` and
> `RENAME`; anything else is reported as `not analyzed` instead of guessed.

## Example

```sql
-- examples/migrations/0002_track_logins.sql
ALTER TABLE users ADD COLUMN last_login_at timestamptz;

CREATE INDEX idx_users_last_login_at ON users (last_login_at);

ALTER TABLE users ADD COLUMN api_token uuid DEFAULT gen_random_uuid();
```

```console
$ lockline check examples/migrations
examples/migrations/0001_create_users.sql
  1:1  CREATE TABLE users ( id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, email …

  7:1  CREATE INDEX idx_users_email ON users (email)
       lock   SHARE on users — blocks writes, allows reads (new table)
       cost   full table scan (index build)

examples/migrations/0002_track_logins.sql
  1:1  ALTER TABLE users ADD COLUMN last_login_at timestamptz
       lock   ACCESS EXCLUSIVE on users — blocks reads and writes
       cost   metadata only (fast)
  warning[missing-lock-timeout] This migration takes an ACCESS EXCLUSIVE lock on "users" without a lock_timeout; if the lock has to wait, every query on the table queues behind it.
       help: Add SET lock_timeout = '5s'; at the start of the migration (SET LOCAL inside a transaction) and retry on failure.

  3:1  CREATE INDEX idx_users_last_login_at ON users (last_login_at)
       lock   SHARE on users — blocks writes, allows reads
       cost   full table scan (index build)
  error[create-index-not-concurrently] Building an index without CONCURRENTLY blocks all writes to "users" until the build finishes.
       help: CREATE INDEX CONCURRENTLY idx_users_last_login_at ON users USING btree (last_login_at);
             (CONCURRENTLY cannot run inside a transaction block)

  5:1  ALTER TABLE users ADD COLUMN api_token uuid DEFAULT gen_random_uuid()
       lock   ACCESS EXCLUSIVE on users — blocks reads and writes
       cost   rewrites the table and its indexes
  error[add-column-rewrite] Adding this column rewrites "users" and all its indexes while holding an ACCESS EXCLUSIVE lock.
       help: Add the column without a default (or with a constant default), then backfill in batches and set the default afterwards.

  holds until commit: ACCESS EXCLUSIVE on users

2 errors, 1 warning in 2 files (5 statements)
```

The exit code is `1` when there are errors, so it can gate a pull request.

## Install

From source (requires Rust 1.85+ and a C compiler):

```bash
cargo install --git https://github.com/<you>/lockline
```

Prebuilt binaries and a GitHub Action are planned (milestone 4).

## Usage

```bash
lockline check migrations/                    # every *.sql file, recursively, in natural order
lockline check db/0007_add_index.sql          # a single file
cat migration.sql | lockline check -          # stdin
lockline check migrations/ --quiet            # only statements with findings
lockline check migrations/ --fail-on warning  # also fail on warnings
lockline check migrations/ --transaction none # files run in autocommit (psql -f)
lockline check migrations/ --format json      # machine-readable output
lockline rules                                # every rule and its severity
lockline explain set-not-null                 # why a rule matters and what to do instead
```

| Exit code | Meaning |
|---|---|
| `0` | No findings at or above `--fail-on` (default: `error`) |
| `1` | Findings at or above `--fail-on` |
| `2` | Usage, input or I/O error |

By default each file is assumed to run inside a single transaction, which is how Laravel
(on PostgreSQL), Flyway, Rails and Django run migrations.

## How it understands context

- **New tables are safe.** Statements on a table created earlier in the same migration are
  not flagged: it has no rows and no traffic yet. Their lock is still shown, marked
  `(new table)`.
- **Transactions matter.** `CREATE INDEX CONCURRENTLY` and `VACUUM` fail inside a
  transaction block; `BEGIN`, `COMMIT` and `ROLLBACK` in the file are tracked.
- **Locks pile up until COMMIT.** When a transaction has several statements, lockline
  prints every lock it holds until it commits (`holds until commit: ...`).
- **`lock_timeout` is tracked.** A migration that takes an ACCESS EXCLUSIVE lock without
  `SET lock_timeout` first gets one warning per file.
- **It remembers the schema of the run.** Column types, validated `CHECK (col IS NOT NULL)`
  constraints and index names from earlier files are used to recognize safe type widenings,
  `SET NOT NULL` that skips the table scan, and the table behind `DROP INDEX`.
- **It never guesses.** Statements outside the lock catalog are reported as
  `not analyzed`.

## Rules

Each rule links to its page, which `lockline explain <rule>` also prints.

| Rule | Severity | What it catches |
|---|---|---|
| [`create-index-not-concurrently`](docs/rules/create-index-not-concurrently.md) | error | Flags `CREATE INDEX` without `CONCURRENTLY` on a table that already exists. |
| [`cannot-run-in-transaction`](docs/rules/cannot-run-in-transaction.md) | error | Flags statements PostgreSQL refuses to run inside a transaction block: `CREATE`/`DROP INDEX CONCURRENTLY`, `REINDEX CONCURRENTLY`, `DETACH PARTITION CONCURRENTLY` and `VACUUM`. |
| [`add-column-rewrite`](docs/rules/add-column-rewrite.md) | error | Flags `ADD COLUMN` that forces PostgreSQL to write a value into every existing row. |
| [`drop-index-not-concurrently`](docs/rules/drop-index-not-concurrently.md) | warning | Flags `DROP INDEX` without `CONCURRENTLY`. |
| [`reindex-not-concurrently`](docs/rules/reindex-not-concurrently.md) | error | Flags `REINDEX TABLE` or `REINDEX INDEX` without `CONCURRENTLY`. |
| [`rewrite-table`](docs/rules/rewrite-table.md) | error | Flags `VACUUM FULL` and `CLUSTER` on existing tables. |
| [`refresh-matview-not-concurrently`](docs/rules/refresh-matview-not-concurrently.md) | warning | Flags `REFRESH MATERIALIZED VIEW` without `CONCURRENTLY`. |
| [`backfill-in-migration`](docs/rules/backfill-in-migration.md) | warning | Flags `UPDATE` or `DELETE` without `WHERE` on an existing table. |
| [`add-column-not-null-without-default`](docs/rules/add-column-not-null-without-default.md) | error | Flags `ADD COLUMN ... NOT NULL` without a `DEFAULT` on an existing table. |
| [`add-column-unknown-default`](docs/rules/add-column-unknown-default.md) | warning | Flags `ADD COLUMN ... DEFAULT f()` where lockline cannot tell whether `f` is volatile. |
| [`alter-column-type`](docs/rules/alter-column-type.md) | error | Flags `ALTER COLUMN ... TYPE` that rewrites the table. |
| [`set-not-null`](docs/rules/set-not-null.md) | error | Flags `ALTER COLUMN ... SET NOT NULL` on an existing table. |
| [`add-check-without-not-valid`](docs/rules/add-check-without-not-valid.md) | error | Flags `ADD CONSTRAINT ... CHECK` without `NOT VALID` on an existing table. |
| [`add-foreign-key-without-not-valid`](docs/rules/add-foreign-key-without-not-valid.md) | error | Flags a new foreign key without `NOT VALID` on an existing table. |
| [`add-unique-without-index`](docs/rules/add-unique-without-index.md) | error | Flags `ADD CONSTRAINT ... UNIQUE` or `PRIMARY KEY` that builds its index inside the `ALTER TABLE`. |
| [`missing-lock-timeout`](docs/rules/missing-lock-timeout.md) | warning | Flags a migration that takes an ACCESS EXCLUSIVE lock without setting `lock_timeout` first. Reported once per file. |
| [`rename-object`](docs/rules/rename-object.md) | warning | Flags renaming a table or a column that already exists. |
| [`drop-object`](docs/rules/drop-object.md) | warning | Flags dropping a table or a column that already exists. |
| [`parse-error`](docs/rules/parse-error.md) | error | The file is not valid PostgreSQL 17 SQL, so lockline cannot analyze any of it. |
| [`ignore-without-reason`](docs/rules/ignore-without-reason.md) | warning | Flags a `lockline:ignore` comment that does not say why the finding is safe to ignore. |

## Suppressing a finding

Put a full-line comment with the rule and the reason right before the statement:

```sql
-- lockline:ignore create-index-not-concurrently the table has under 1000 rows
CREATE INDEX idx_flags_name ON flags (name);
```

`-- lockline:ignore-file <rule> <reason>` applies to the whole file. Several rules can be
listed separated by commas. A suppression without a reason still works, but raises
`ignore-without-reason`. Suppressed findings are counted in the summary.

## Configuration

`lockline.toml` in the current directory (or any parent) sets project defaults.
Command-line flags win over it.

```toml
transaction = "per-unit"            # or "none"
fail_on = "error"                   # or "warning"
exclude = ["migrations/legacy/**"]  # glob patterns, matched against the file path
[rules]
drop-object = "off"                 # off | warning | error
missing-lock-timeout = "error"
```

Unknown keys and unknown rule ids are errors (exit code 2), so a typo never silently
disables a rule.

## JSON output

`--format json` prints a stable document (`"version": 1`) with every file, statement,
effect (table, lock, `blocks_reads`, `blocks_writes`, cost), finding, suppression and held
lock, plus a summary. The exit code is the same as in the human format.

## Prior art

lockline stands on the shoulders of great tools, and you should look at them too:

- [squawk](https://github.com/sbdchd/squawk) — a mature PostgreSQL migration linter with many rules.
- [eugene](https://github.com/kaaveland/eugene) — traces the locks a migration takes by running it against a database.
- [strong_migrations](https://github.com/ankane/strong_migrations) — catches unsafe migrations in Rails.

lockline's angle is different: it **explains the lock of every statement** (not only the
ones that break a rule), it **understands the migration's context** (new tables,
transactions), and its lock catalog will be **verified against real PostgreSQL 14–17 in
CI** (milestone 3). It will also read Laravel's `migrate --pretend` output directly.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
