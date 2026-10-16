# lockline

**Know what your PostgreSQL migration locks before it hits production.**

A migration that takes 20 ms on your laptop can block every write to a 50M-row table for
minutes in production. `lockline` reads your migration SQL with the real PostgreSQL parser
and tells you, for every statement, which lock it takes, what that lock blocks, and whether
it rewrites or scans the table. When a pattern is dangerous, it explains why and suggests
the safe version.

> **Status:** early development (milestone 1 of 4). Index and `ADD COLUMN` statements are
> analyzed; other statements are reported as `not analyzed` instead of guessed.

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
lockline check migrations/                    # every *.sql file, recursively, in path order
lockline check db/0007_add_index.sql          # a single file
cat migration.sql | lockline check -          # stdin
lockline check migrations/ --quiet            # only statements with findings
lockline check migrations/ --fail-on warning  # also fail on warnings
lockline check migrations/ --transaction none # files run in autocommit (psql -f)
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
- **It never guesses.** Statements outside the lock catalog are reported as
  `not analyzed`.

## Rules

| Rule | Severity | What it catches |
|---|---|---|
| `create-index-not-concurrently` | error | `CREATE INDEX` without `CONCURRENTLY` on an existing table blocks writes for the whole build |
| `cannot-run-in-transaction` | error | `CONCURRENTLY` operations or `VACUUM` inside a transaction block: the migration will fail |
| `add-column-rewrite` | error | `ADD COLUMN` with a volatile default, `serial`, identity or stored generated column rewrites the table |
| `parse-error` | error | The file is not valid PostgreSQL |

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
