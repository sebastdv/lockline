# lockline

**Know what your PostgreSQL migration locks before it hits production.**

A migration that takes 20 ms on your laptop can block every write to a 50M-row table for
minutes in production. `lockline` reads your migration SQL with the real PostgreSQL parser
and tells you, for every statement, which lock it takes, what that lock blocks, and whether
it rewrites or scans the table. When a pattern is dangerous, it explains why and suggests
the safe version.

> **Status:** v0.1, feature-complete. The locks, rewrites and "metadata only"
> claims for the statements in `tests/pg_verify.rs` are verified against PostgreSQL 14, 15,
> 16 and 17 (see [known gaps](#known-gaps)). The lock catalog covers `ALTER TABLE`,
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

From source (requires Rust 1.88+ and a C compiler):

```bash
cargo install --git https://github.com/sebastdv/lockline
```

Prebuilt binaries for macOS (Apple Silicon and Intel) and Linux (x86_64 and arm64) are
attached to each [release](https://github.com/sebastdv/lockline/releases), with a SHA-256
checksum:

```bash
target=aarch64-apple-darwin   # or x86_64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
curl -fsSLO "https://github.com/sebastdv/lockline/releases/latest/download/lockline-$target.tar.gz"
tar -xzf "lockline-$target.tar.gz" && sudo mv "lockline-$target/lockline" /usr/local/bin/
```

In GitHub Actions, use the [action](#in-ci) instead.

## Usage

```bash
lockline check migrations/                    # every *.sql file, recursively, in natural order
lockline check db/0007_add_index.sql          # a single file
cat migration.sql | lockline check -          # stdin
lockline check migrations/ --quiet            # only statements with findings
lockline check migrations/ --fail-on warning  # also fail on warnings
lockline check migrations/ --transaction none # files run in autocommit (psql -f)
lockline check migrations/ --format json      # machine-readable output
lockline check migrations/ --format sarif     # SARIF 2.1.0, for GitHub code scanning
lockline check migrations/ --format github    # GitHub Actions annotations
lockline check migrations/0042_new.sql --history migrations/   # learn the schema from earlier files
php artisan migrate --pretend | lockline check --from laravel -
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
| [`ignore-unknown-rule`](docs/rules/ignore-unknown-rule.md) | warning | Flags a `lockline:ignore` comment that names a rule lockline does not have. |

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

## Laravel

lockline reads the SQL Laravel would run, so it sees exactly what the schema builder
generates (`->change()`, `->index()`, `->foreign()`…):

```bash
php artisan migrate --pretend | lockline check --from laravel -
```

Each Laravel migration is one unit, named after its file
(`database/migrations/2026_10_17_000000_add_nickname_to_users.php`, or under
`--laravel-dir`), so annotations land on the PHP file. Positions in the report count the
statements of the migration, since the PHP file has no SQL lines. Both the Laravel 9+ output
(`⇂` lines) and the older `ClassName: statement` format are understood. `migrate --pretend`
needs a database connection to know which migrations are pending.

`->change()` repeats `SET NOT NULL` on columns that already are NOT NULL; lockline knows
PostgreSQL does not scan the table in that case and does not flag it. To give it the schema
of migrations that already ran, pass Laravel's schema dump:

```bash
php artisan migrate --pretend | lockline check --from laravel - --history database/schema/pgsql-schema.sql
```

## In CI

The action installs a verified release binary and annotates the pull request:

```yaml
- uses: sebastdv/lockline@v0.1.0
  with:
    path: migrations/          # or: from: laravel, path: pretend.txt
    history: migrations/       # optional: schema context from earlier files
    fail-on: error             # or warning
    sarif: "true"              # optional: also write lockline.sarif
- uses: github/codeql-action/upload-sarif@v3   # optional, needs `security-events: write`
  if: always()
  with:
    sarif_file: lockline.sarif
```

To check only the migrations a pull request adds, pass those files as `path` and the
migrations directory as `history`: earlier files are read only to learn the schema (column
types, NOT NULL columns, validated checks) and are never reported.

`version: source` builds the action's own checkout with cargo instead of downloading a
release; this repository's [dogfood workflow](.github/workflows/dogfood.yml) uses it on
`examples/migrations`.

## Output formats

- `human` (default): every statement with its lock, and the findings.
- `json`: a stable document (`"version": 1`) with every file, statement, effect (table,
  lock, `blocks_reads`, `blocks_writes`, cost), finding, suppression and held lock, plus a
  summary.
- `sarif`: SARIF 2.1.0 with one rule per lockline rule (its documentation included), the
  findings, and suppressed findings marked as suppressed in source with their reason.
- `github`: workflow commands that GitHub shows as annotations on the changed files.

The exit code is the same in every format.

## Prior art

lockline stands on the shoulders of great tools, and you should look at them too:

- [squawk](https://github.com/sbdchd/squawk) — a mature PostgreSQL migration linter with many rules.
- [eugene](https://github.com/kaaveland/eugene) — traces the locks a migration takes by running it against a database.
- [strong_migrations](https://github.com/ankane/strong_migrations) — catches unsafe migrations in Rails.

lockline's angle is different: it **explains the lock of every statement** (not only the
ones that break a rule), it **understands the migration's context** (new tables,
transactions, the schema of earlier files), and its lock catalog is **checked against real
PostgreSQL 14–17** by a test suite. It also reads Laravel's `migrate --pretend` output
directly.

## How the lock catalog is verified

`tests/pg_verify.rs` runs about 85 statements against a real PostgreSQL and compares what
happens with what lockline claims:

- **Locks:** each statement runs inside `BEGIN … ROLLBACK` and the test reads its own locks
  from `pg_locks`. Statements that cannot run in a transaction (`CONCURRENTLY`, `VACUUM`) are
  observed from a second session: it holds a lock on one table and the test reads the lock
  the statement waits for there, one table per case.
- **Rewrites:** a table is rewritten when its `pg_class.relfilenode` changes.
- **"Metadata only":** the changed table must not be read with a sequential scan
  (`pg_stat_xact_user_tables`).
- **Omissions:** a lock on an existing table that the catalog does not mention fails the test.

The expected values come from lockline's own catalog, so the test checks exactly what the
tool prints. When PostgreSQL disagrees, the catalog is fixed. It has been run locally on
PostgreSQL 14, 15, 16 and 17, and a CI job runs it on all four:

```bash
docker compose up -d --wait
LOCKLINE_TEST_DATABASE_URL=postgres://lockline:lockline@localhost:54317/lockline cargo test --test pg_verify
```

Use `PG_VERSION=14 docker compose up -d --force-recreate --wait` to try another version.

### Known gaps

- Locks on **partitions and inheritance children** of the altered table (for example
  `ALTER TABLE parent ADD COLUMN` also locks every partition) and on a **default partition**
  during `ATTACH PARTITION` are not reported yet.
- **DML on tables with foreign keys** also takes ROW SHARE on the related tables; lockline
  reports only the table being written.
- Locks that depend on objects created outside the analyzed files (an index or a foreign key
  lockline never saw) cannot be resolved; lockline says so instead of guessing.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
