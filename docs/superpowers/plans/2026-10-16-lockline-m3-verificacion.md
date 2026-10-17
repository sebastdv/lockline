# lockline — Hito 3 (verificación) · Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cada lock y cada reescritura que afirma el catálogo queda comprobada contra PostgreSQL 14–17 reales, en local y en CI. Además se cierran los 11 detalles menores que dejó la revisión del hito 2.

**Architecture:** `tests/pg_verify.rs` crea un esquema aislado para cada caso y ejecuta un *setup* común. Después corre la sentencia y observa sus locks de dos formas:
- **En transacción:** `BEGIN`, la sentencia, leer `pg_locks` de la propia sesión y `ROLLBACK`.
- **Bloqueada:** para sentencias que no admiten transacción, una segunda conexión retiene la tabla y se lee el lock que la sentencia espera.

Lo esperado no se escribe a mano: sale de `lockline::catalog::analyze_statement`. Así el test verifica exactamente lo que la herramienta afirma. "Reescribe" se comprueba con el cambio de `pg_class.relfilenode`. Si Postgres contradice al catálogo, gana Postgres.

**Tech Stack:** lo de los hitos 1–2, más `postgres` 0.19 (dev-dependency, cliente síncrono) y Docker (`docker-compose.yml`) en local. En CI, servicios de GitHub Actions con Postgres 14, 15, 16 y 17.

**Spec:** `docs/superpowers/specs/2026-10-16-lockline-design.md` (§5 catálogo, §8 verificación, §10 hito 3). Planes anteriores: `docs/superpowers/plans/2026-10-16-lockline-m1-nucleo.md` y `…-m2-cobertura.md`.

## Global Constraints

- Lo de los hitos anteriores sigue vigente:
  - `cargo fmt --check` y `cargo clippy --all-targets -- -D warnings` limpios.
  - Mensajes en inglés.
  - Nunca inventar un lock.
  - Las reglas no disparan sobre tablas nuevas.
- **Gana Postgres:** si una observación contradice al catálogo, se corrige el catálogo, su test unitario en `tests/catalog.rs` y la fila del spec §5, en el mismo commit.
- Si el lock difiere entre versiones 14–17, el catálogo usa **el más fuerte observado** y la nota del efecto lo dice (`stronger on PostgreSQL 14`, por ejemplo).
- `tests/pg_verify.rs` se salta en silencio sin `LOCKLINE_TEST_DATABASE_URL`, salvo que `LOCKLINE_REQUIRE_PG=1`: en ese caso falla. CI define las dos.
- **En esta máquina:**
  - Docker Desktop lo abre el usuario; el ejecutor le pide que lo haga.
  - El sandbox de Bash no alcanza los puertos publicados, así que los comandos que corren `pg_verify` contra el contenedor se ejecutan con el sandbox desactivado (pidiendo permiso) o los corre el usuario en su terminal.
  - La VM de Docker tiene 4 GB: un solo contenedor a la vez.
- URL local: `postgres://lockline:lockline@localhost:54317/lockline`.
- Commits: Conventional Commits en español, una línea, sin cuerpo ni `Co-Authored-By`. Los hace el ejecutor si el usuario lo pidió (en el hito 2 lo pidió).

## Review Focus

1. **CI sin base de datos por error** (un servicio que no arrancó o una variable mal escrita) → con `LOCKLINE_REQUIRE_PG=1` la suite falla en vez de "pasar" sin verificar nada. Test en Task 1.
2. **Tests en paralelo sobre la misma base** → cada caso usa su propio esquema, y una observación bloqueada nunca cuelga la suite: a los 10 s libera el bloqueo y falla con un mensaje claro. Test en Task 1.
3. **Locks que no son omisiones del catálogo** → no se reportan como "lock extra":
   - locks sobre índices, secuencias o TOAST;
   - `AccessShare` por leer el catálogo;
   - locks sobre la relación que la sentencia crea.
   Test en Task 1.
4. **Diferencias entre versiones 14–17** → la matriz de CI las expone y el catálogo usa el lock más fuerte con nota. Se cubre en Task 5.
5. **`-- lockline:ignore` dentro de un cuerpo `$$` o de un `/* */`** → no suprime nada. Test en Task 6.

---

### Task 1: Arnés de verificación contra Postgres

**Files:**
- Create: `docker-compose.yml`, `tests/pg_verify.rs`
- Modify: `Cargo.toml` (dev: `postgres = "0.19"`), `.gitignore` (nada nuevo)

**Interfaces:**
- Consumes: `lockline::parse::parse_sql`, `lockline::schema::Schema`, `lockline::catalog::analyze_statement`, `lockline::model::{Analysis, Cost, Effect}`, `lockline::locks::LockMode`.
- Produces (privado de `tests/pg_verify.rs`, usado por las Tasks 2–4):
  - `enum Run { InTransaction, Blocking }`
  - `struct Case { extra_setup: &'static str, sql: &'static str, run: Run, block_on: Option<&'static str> }`. `block_on` es la tabla que retiene la conexión B; `None` significa `effects[0]`.
  - Constructores para que las listas de casos sean cortas:
    - `const fn tx(sql) -> Case`
    - `const fn tx_after(extra_setup, sql) -> Case`
    - `const fn blocking(sql) -> Case`
    - `const fn blocking_on(table, sql) -> Case`
    En los mensajes de desacuerdo, el caso se identifica por su `sql`.
  - `const BASE_SETUP: &str` (abajo).
  - `fn resolve_url(url: Option<String>, require: bool) -> Option<String>`: función pura.
    - Sin URL y con `require` ⇒ `panic!("LOCKLINE_TEST_DATABASE_URL is not set but LOCKLINE_REQUIRE_PG=1")`.
    - Sin URL y sin `require` ⇒ imprime `skipped: LOCKLINE_TEST_DATABASE_URL not set` y devuelve `None`.
  - `fn database_url() -> Option<String>`: llama a `resolve_url` con las dos variables de entorno.
  - `struct Observed { locks: BTreeMap<String, LockMode>, rewritten: BTreeSet<String> }`. Las claves son `relname`. Solo cuenta:
    - relaciones que existían **antes** de la sentencia;
    - `relkind` en `r`, `p`, `m`, `v` o `f`;
    - modos más fuertes que `AccessShare`.
    `locks` guarda el modo más fuerte por relación; `rewritten` contiene las relaciones cuyo `relfilenode` cambió.
  - `fn observe(url: &str, case: &Case, schema: &str) -> Result<Observed, String>`:
    1. Crea el esquema `schema` y fija `search_path` a él.
    2. Ejecuta `BASE_SETUP` y luego `case.extra_setup`.
    3. Guarda los `relfilenode` del esquema y observa según `case.run`.
    4. Borra el esquema con `DROP SCHEMA … CASCADE`, pase lo que pase.
  - `fn expected(case: &Case) -> Analysis`: parsea `BASE_SETUP` y `extra_setup` en un `Schema`, y analiza `case.sql`.
  - `fn compare(case: &Case, expected: &Analysis, observed: &Observed) -> Vec<String>`: devuelve los desacuerdos legibles. Reglas:
    - `Locks(effects)`, por cada efecto:
      - `observed.locks[effect.table.name] == effect.lock`;
      - `Cost::Rewrite` ⇔ la tabla está en `rewritten`;
      - `Metadata`/`Scan` ⇒ no está;
      - `Unknown` no se compara.
    - Una relación en `observed.locks` que no esté en los efectos ⇒ desacuerdo `"extra lock {MODE} on {rel}"`.
    - `NoTableLock` ⇒ `observed.locks` vacío.
    - `NotAnalyzed` ⇒ nunca se compara: un caso así es un error del test, salvo en las sondas de la Task 4.
  - `fn verify(cases: &[Case])`:
    - Si `database_url()` es `None`, retorna.
    - Si no, corre cada caso con el esquema `lv_{índice}_{pid del proceso}` y junta todos los desacuerdos.
    - Termina con `panic!` mostrando la lista completa, para que una sola corrida enseñe todo lo que falla.

`BASE_SETUP` (fijo; los casos usan estos nombres):
```sql
CREATE TABLE orgs (id bigint PRIMARY KEY);
CREATE TABLE users (id bigint PRIMARY KEY, email text, name varchar(100), age int, org_id bigint);
CREATE INDEX users_email_idx ON users (email);
CREATE TABLE orders (id bigint PRIMARY KEY, user_id bigint, created_at timestamptz);
CREATE TABLE events (id bigint, at date) PARTITION BY RANGE (at);
CREATE TABLE events_2025 PARTITION OF events FOR VALUES FROM ('2025-01-01') TO ('2026-01-01');
CREATE TABLE events_2026 (id bigint, at date);
CREATE MATERIALIZED VIEW stats AS SELECT 1 AS n;
CREATE UNIQUE INDEX stats_n_idx ON stats (n);
CREATE VIEW v AS SELECT 1 AS n;
CREATE TABLE plain (id int);
CREATE FUNCTION touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;
CREATE TRIGGER users_touch BEFORE INSERT ON users FOR EACH ROW EXECUTE FUNCTION touch();
INSERT INTO orgs SELECT g FROM generate_series(1, 3) g;
INSERT INTO users SELECT g, 'u' || g, 'n', 20, 1 FROM generate_series(1, 10) g;
INSERT INTO plain SELECT g FROM generate_series(1, 10) g;
```

Observación **en transacción**, por la conexión A:
1. `BEGIN`.
2. La sentencia.
3. ```sql
   SELECT c.relname, l.mode
   FROM pg_locks l JOIN pg_class c ON c.oid = l.relation
   WHERE l.pid = pg_backend_pid() AND l.locktype = 'relation'
     AND c.relnamespace = '{schema}'::regnamespace
   ```
4. Los `relfilenode` dentro de la transacción.
5. `ROLLBACK`.

Observación **bloqueada** (relación objetivo = `case.block_on` o, si es `None`, `effects[0]` de `expected`):
1. La conexión B hace `BEGIN; LOCK TABLE {target} IN ACCESS EXCLUSIVE MODE`.
2. La conexión A obtiene su `pg_backend_pid()` y, en un hilo, ejecuta la sentencia, que queda esperando.
3. La conexión C sondea cada 50 ms, hasta 10 s, `pg_locks WHERE pid = {A} AND NOT granted AND relation = '{target}'::regclass` y toma ese `mode`.
4. B hace `ROLLBACK`, se espera el hilo y se comparan los `relfilenode`.
5. Si pasan 10 s sin observarlo: B hace `ROLLBACK` y se devuelve `Err("timed out waiting for … to block")`.

Mapa de modos de `pg_locks` a `LockMode`: `AccessShareLock`, `RowShareLock`, `RowExclusiveLock`, `ShareUpdateExclusiveLock`, `ShareLock`, `ShareRowExclusiveLock`, `ExclusiveLock` y `AccessExclusiveLock`.

`docker-compose.yml`:
- servicio `postgres`, imagen `postgres:${PG_VERSION:-17}`;
- puerto `54317:5432`;
- variables `POSTGRES_USER`, `POSTGRES_PASSWORD` y `POSTGRES_DB` en `lockline`;
- healthcheck con `pg_isready -U lockline`.

- [ ] **Step 1: Tests que fallan** (en `tests/pg_verify.rs`)
  - `harness_cases`: `verify(&[…])` con:
    - `CREATE INDEX i ON users (email)` (InTransaction);
    - `ALTER TABLE users ADD COLUMN token uuid DEFAULT gen_random_uuid()` (InTransaction; comprueba `rewritten`);
    - `CREATE INDEX CONCURRENTLY i2 ON users (name)` (Blocking).
  - `extra_locks_ignore_indexes_and_reads` (Review Focus 3): `INSERT INTO users VALUES (99, 'x', 'n', 1, 1)` toma RowExclusive sobre `users` y sus índices, más AccessShare de lectura. Esperado: sin desacuerdos; los índices no cuentan como lock extra.
  - `require_pg_fails_without_url` (Review Focus 1): `#[should_panic(expected = "LOCKLINE_REQUIRE_PG=1")]` sobre `resolve_url(None, true)`. Además, `resolve_url(None, false) == None` y `resolve_url(Some(u), true) == Some(u)`.
  - `blocking_times_out_cleanly` (Review Focus 2): `blocking_on("users", "SELECT 1")`, que nunca se bloquea ⇒ `observe` devuelve `Err` que contiene `timed out` en menos de 12 s. La conexión B se libera, así que el siguiente caso no queda bloqueado.
- [ ] **Step 2: Levantar Postgres** — pedir al usuario que abra Docker Desktop. Run: `docker compose up -d --wait`. Expected: `postgres` healthy.
- [ ] **Step 3: Verificar que fallan** — Run (sandbox desactivado): `LOCKLINE_TEST_DATABASE_URL=postgres://lockline:lockline@localhost:54317/lockline cargo test --test pg_verify` · Expected: FAIL (no compila).
- [ ] **Step 4: Implementar el arnés.**
- [ ] **Step 5: Verificar** — mismo comando · Expected: PASS. Además, `cargo test` sin la variable ⇒ PASS con `skipped` en la salida.
- [ ] **Step 6: Commit**
```bash
git add Cargo.toml Cargo.lock docker-compose.yml tests/pg_verify.rs
git commit -m "test(verify): agregar arnés que compara el catálogo con los locks de postgres real"
```

---

### Task 2: Verificar el catálogo de `ALTER TABLE`

**Files:**
- Modify: `tests/pg_verify.rs`; y según los resultados `src/catalog.rs`, `tests/catalog.rs`, `docs/superpowers/specs/2026-10-16-lockline-design.md`

**Interfaces:**
- Consumes: `Case`, `verify` (Task 1).

Casos (todos `InTransaction`), en un test `alter_table_matches_postgres`. `extra_setup` vacío salvo donde se indica:

| Sentencia | `extra_setup` |
|---|---|
| `ALTER TABLE users ADD COLUMN nick text` | |
| `… ADD COLUMN created timestamptz DEFAULT now()` | |
| `… ADD COLUMN n bigserial` | |
| `… ADD COLUMN ident int GENERATED ALWAYS AS IDENTITY` | |
| `… ADD COLUMN twice int GENERATED ALWAYS AS (age * 2) STORED` | |
| `… ADD COLUMN code text UNIQUE` | |
| `… ADD COLUMN c2 int CHECK (c2 > 0)` | |
| `… ADD COLUMN org2 bigint REFERENCES orgs (id)` | |
| `… DROP COLUMN name` | |
| `… ALTER COLUMN age SET DEFAULT 1` / `DROP DEFAULT` / `DROP NOT NULL` | |
| `… ALTER COLUMN age TYPE bigint` | |
| `… ALTER COLUMN name TYPE varchar(200)` (ampliación: Metadata) | |
| `… ALTER COLUMN name TYPE varchar(50)` | |
| `… ALTER COLUMN age SET NOT NULL` | |
| `… ALTER COLUMN age SET NOT NULL` (check validado previo: Metadata) | `ALTER TABLE users ADD CONSTRAINT age_nn CHECK (age IS NOT NULL);` |
| `… ADD CONSTRAINT c CHECK (age > 0)` y la variante `NOT VALID` | |
| `… ADD CONSTRAINT fk FOREIGN KEY (org_id) REFERENCES orgs (id)` y `NOT VALID` | |
| `… ADD CONSTRAINT u UNIQUE (email)` | |
| `… ADD CONSTRAINT u UNIQUE USING INDEX users_email_uidx` | `CREATE UNIQUE INDEX users_email_uidx ON users (email);` |
| `ALTER TABLE logs ADD CONSTRAINT logs_pkey PRIMARY KEY USING INDEX logs_id_idx` | `CREATE TABLE logs (id bigint NOT NULL); CREATE UNIQUE INDEX logs_id_idx ON logs (id);` |
| `… VALIDATE CONSTRAINT age_pos` | `ALTER TABLE users ADD CONSTRAINT age_pos CHECK (age > 0) NOT VALID;` |
| `… VALIDATE CONSTRAINT users_org_fk` | `ALTER TABLE users ADD CONSTRAINT users_org_fk FOREIGN KEY (org_id) REFERENCES orgs (id) NOT VALID;` |
| `… ALTER COLUMN age SET STATISTICS 100`, `SET (n_distinct = 10)`, `RESET (n_distinct)` | |
| `… SET (fillfactor = 70)`, `RESET (fillfactor)` | |
| `… CLUSTER ON users_pkey`, `SET WITHOUT CLUSTER` | |
| `ALTER TABLE idents ALTER COLUMN id SET GENERATED BY DEFAULT`, `DROP IDENTITY` | `CREATE TABLE idents (id int GENERATED ALWAYS AS IDENTITY);` |
| `ALTER TABLE plain ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY` | `ALTER TABLE plain ALTER COLUMN id SET NOT NULL;` |
| `… REPLICA IDENTITY FULL`, `OWNER TO CURRENT_USER`, `ENABLE ROW LEVEL SECURITY`, `FORCE ROW LEVEL SECURITY` | |
| `ALTER TABLE orders ALTER CONSTRAINT orders_user_fk DEFERRABLE` | `ALTER TABLE orders ADD CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES users (id);` |
| `… ENABLE TRIGGER ALL`, `DISABLE TRIGGER users_touch` | |
| `ALTER TABLE plain SET UNLOGGED`, `SET LOGGED` (este con `extra_setup` `ALTER TABLE plain SET UNLOGGED;`) | |
| `ALTER TABLE events ATTACH PARTITION events_2026 FOR VALUES FROM ('2026-01-01') TO ('2027-01-01')` | |
| `ALTER TABLE events DETACH PARTITION events_2025` | |
| `… RENAME TO people`, `RENAME COLUMN email TO mail`, `RENAME CONSTRAINT users_pkey TO users_pk` | |

Desacuerdos que ya se esperan, según la revisión del hito 2. Si Postgres los confirma, se corrigen así:
- `VALIDATE CONSTRAINT` de una FK: falta RowShare sobre la tabla referenciada. Se agrega el efecto; la referenciada sale de una nueva entrada del esquema virtual: `Schema` registra todas las FK por nombre de constraint, con su tabla referenciada, mediante `pub fn foreign_key_target(&self, table: &TableRef, constraint: &str) -> Option<&TableRef>`. Así sirve también si `ALTER CONSTRAINT` sobre una FK resulta tomar locks en la referenciada.
- `DETACH PARTITION`: falta AccessExclusive (o ShareUpdateExclusive si es `CONCURRENTLY`) sobre la partición. Se agrega el efecto extra.
- `ALTER COLUMN TYPE` con una tabla que tiene índices: Postgres también reescribe los índices. `rewritten` solo mira tablas, así que no cambia nada.

- [ ] **Step 1:** escribir `alter_table_matches_postgres` con todos los casos.
- [ ] **Step 2:** correrlo (sandbox desactivado). Expected: FAIL con la lista de desacuerdos. Copiarla al ledger.
- [ ] **Step 3:** por cada desacuerdo, primero un test unitario en `tests/catalog.rs` con el valor observado (que debe fallar), luego el arreglo en `src/catalog.rs` (y en `src/schema.rs` si hace falta) y la fila del spec.
- [ ] **Step 4:** volver a correr `alter_table_matches_postgres` y `cargo test`. Expected: PASS.
- [ ] **Step 5: Commit**
```bash
git add tests src docs/superpowers/specs
git commit -m "test(verify): verificar el catálogo de alter table contra postgres y corregir los locks observados"
```

---

### Task 3: Verificar el resto del catálogo

**Files:** como en la Task 2.

Casos en `statements_match_postgres` (InTransaction) y `non_transactional_match_postgres` (Blocking):

| Sentencia | Run | `extra_setup` |
|---|---|---|
| `CREATE INDEX i ON users (email)` | Tx | |
| `DROP TABLE orders` | Tx | |
| `DROP INDEX users_email_idx` | Tx | |
| `TRUNCATE orders` | Tx | |
| `REINDEX TABLE users`, `REINDEX INDEX users_email_idx` | Tx | |
| `ANALYZE users` | Tx | |
| `CLUSTER users USING users_pkey` | Tx | |
| `REFRESH MATERIALIZED VIEW stats`, `… CONCURRENTLY stats` | Tx | |
| `CREATE TRIGGER t2 AFTER UPDATE ON users FOR EACH ROW EXECUTE FUNCTION touch()` | Tx | |
| `INSERT INTO users VALUES (50, 'x', 'n', 1, 1)`, `UPDATE users SET age = 2`, `DELETE FROM users WHERE id = 1` | Tx | |
| `CREATE TABLE events_2027 PARTITION OF events FOR VALUES FROM ('2027-01-01') TO ('2028-01-01')` | Tx | |
| `CREATE TABLE kids (extra int) INHERITS (plain)` | Tx | |
| `CREATE TABLE memberships (org_id bigint REFERENCES orgs (id))` | Tx | |
| `CREATE TABLE fresh (id int)` (NoTableLock) | Tx | |
| `CREATE OR REPLACE VIEW v AS SELECT 1 AS n` | Tx | |
| `COMMENT ON TABLE users IS 'x'`, `COMMENT ON COLUMN users.email IS 'x'` | Tx | |
| `CREATE INDEX CONCURRENTLY i3 ON users (age)` | Blocking | |
| `DROP INDEX CONCURRENTLY users_email_idx` | Blocking | |
| `REINDEX TABLE CONCURRENTLY users` | Blocking | |
| `ALTER TABLE events DETACH PARTITION events_2025 CONCURRENTLY` | Blocking | |
| `VACUUM users`, `VACUUM FULL users` | Blocking | |

Desacuerdo que ya se espera: `REFRESH MATERIALIZED VIEW CONCURRENTLY` aplica el diff en el lugar, así que el `relfilenode` no cambia. Si se confirma, su costo pasa a `Scan`, con la nota `computes the new contents and applies the difference`. `rewrite-table` y `refresh-matview-not-concurrently` no dependen de ese costo, pero se revisan sus tests.

- [ ] Pasos 1–5 como en la Task 2. Commit:
```bash
git add tests src docs/superpowers/specs
git commit -m "test(verify): verificar índices, mantenimiento, dml y ddl de tablas contra postgres"
```

---

### Task 4: Sondas y modelado de subcomandos pendientes

Resuelve los pendientes #8 (`SET (user_catalog_table)`) y #11 del hito 2.

**Files:** `tests/pg_verify.rs`, `src/catalog.rs`, `tests/catalog.rs`, spec §5.

**Interfaces:**
- Produces: `fn probe(cases: &[Case]) -> Vec<(String, Observed)>`. Imprime con `--nocapture` el `Observed` de cada caso, sin comparar.

Sentencias a sondear:
- `ALTER TABLE users ALTER COLUMN email SET STORAGE EXTERNAL`
- `ALTER TABLE users ALTER COLUMN email SET COMPRESSION pglz`
- `ALTER TABLE plain ENABLE RULE plain_rule` y `DISABLE RULE plain_rule`, con `extra_setup` `CREATE RULE plain_rule AS ON INSERT TO plain DO INSTEAD NOTHING;`
- `ALTER TABLE users SET (user_catalog_table = true)`
- `ALTER TABLE users SET (fillfactor = 70, user_catalog_table = true)`

- [ ] **Step 1:** test `#[ignore] probe_unmodeled` que llama a `probe` con esos casos.
- [ ] **Step 2:** Run `… cargo test --test pg_verify probe_unmodeled -- --ignored --nocapture` en las 4 versiones (`PG_VERSION=14 docker compose up -d --force-recreate --wait`, y así con 15, 16 y 17). Anotar en el ledger el lock y si hubo reescritura, por versión.
- [ ] **Step 3:** por cada sentencia, un test unitario en `tests/catalog.rs` con lo observado (que falla), y la fila en `alter_cmd_effect`.
  - `SET (…)` toma el modo más fuerte entre sus opciones: `user_catalog_table` según lo observado; el resto, ShareUpdateExclusive como hoy.
  - Para leer las opciones del `AtSetRelOptions`, recorrer su `def` (una lista de `DefElem`).
- [ ] **Step 4:** pasar esas sentencias de la sonda a `alter_table_matches_postgres` (Task 2) para que queden verificadas. Run de `pg_verify` completo y `cargo test` ⇒ PASS.
- [ ] **Step 5: Commit**
```bash
git add tests src docs/superpowers/specs
git commit -m "feat(catalog): modelar set storage, compresión, reglas y user_catalog_table según lo observado en postgres"
```

---

### Task 5: Matriz de Postgres en CI y README

**Files:**
- Modify: `.github/workflows/ci.yml`, `README.md`, `README.es.md`, spec §8

- [ ] **Step 1:** job `pg-verify` en `ci.yml`:
  - `runs-on: ubuntu-latest`, matriz `pg: [14, 15, 16, 17]`, `fail-fast: false`.
  - Servicio `postgres` con imagen `postgres:${{ matrix.pg }}`, `POSTGRES_PASSWORD: lockline`, puerto `5432:5432` y healthcheck `pg_isready`.
  - Variables `LOCKLINE_TEST_DATABASE_URL: postgres://postgres:lockline@localhost:5432/postgres` y `LOCKLINE_REQUIRE_PG: "1"`.
  - Pasos: checkout, toolchain, cache y `cargo test --test pg_verify -- --nocapture`.
- [ ] **Step 2:** correr `pg_verify` en local contra las 4 versiones (`PG_VERSION=… docker compose up -d --force-recreate --wait`). Si una versión difiere: lock más fuerte y nota (Global Constraints), con su test unitario. Expected final: PASS en 14, 15, 16 y 17.
- [ ] **Step 3: README:**
  - Sección "How the lock catalog is verified": qué hace `tests/pg_verify.rs`, cómo correrlo en local (`docker compose up -d --wait` + la variable) y el badge del job.
  - El estado pasa a "milestone 3 of 4".
  - En "Prior art", el futuro "will be verified" pasa a presente.
  - Una línea equivalente en `README.es.md`.
- [ ] **Step 4:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` ⇒ verde.
- [ ] **Step 5: Commit**
```bash
git add .github README.md README.es.md docs/superpowers/specs
git commit -m "ci(verify): verificar el catálogo contra postgres 14 a 17 en github actions"
```

---

### Task 6: Supresiones con el scanner y reglas desconocidas

Pendientes que resuelve: #1 y #2 del hito 2.

**Files:**
- Modify: `src/suppress.rs`, `src/analyze.rs`, `src/rules/mod.rs`, `tests/suppress.rs`, `tests/docs.rs`, `tests/cli.rs`, `README.md`
- Create: `docs/rules/ignore-unknown-rule.md`

**Interfaces:**
- `parse_ignores(sql)` mantiene su firma, pero recorre los tokens `SqlComment` de `pg_query::scan`. Un comentario cuenta solo si todo lo que lo precede en su línea es espacio en blanco. Si `scan` falla, se vuelve a la lectura por líneas actual.
- Nueva pseudo-regla `ignore-unknown-rule` (warning), registrada en `infos()` después de `ignore-without-reason`.
  - Mensaje: `lockline:ignore names an unknown rule "{id}"; run "lockline rules" to list them.`
  - Va en el mismo lugar que `ignore-without-reason`: en la sentencia a la que aplica o, si es de archivo, en la unidad.
- Pasan a 21 reglas: actualizar `ALL_IDS` en `tests/docs.rs`, el conteo de `rules_lists_every_rule` en `tests/cli.rs` y la tabla del README (regenerarla desde `lockline rules`).

- [ ] **Step 1: Tests que fallan**
  - `ignore_inside_dollar_body_does_not_count` (Review Focus 5): `CREATE FUNCTION f() RETURNS void LANGUAGE sql AS $$\n-- lockline:ignore-file drop-object hidden\nSELECT 1;\n$$;\nDROP TABLE users;` ⇒ `drop-object` sigue y `suppressed == 0`.
  - `ignore_inside_block_comment_does_not_count`: `/*\n-- lockline:ignore create-index-not-concurrently x\n*/\nCREATE INDEX i ON users (e);` ⇒ el hallazgo sigue.
  - `unknown_rule_in_ignore_warns`: `-- lockline:ignore drop-objet typo\nDROP TABLE users;` ⇒ hallazgos `drop-object` e `ignore-unknown-rule`, con el mensaje exacto.
  - Actualizar los tests de conteo a 21.
- [ ] Pasos 2–4: `cargo test --test suppress --test docs --test cli` ⇒ FAIL; implementar y escribir la doc (mismo formato que las otras 20); luego `cargo test` ⇒ PASS.
- [ ] **Step 5: Commit**
```bash
git add src tests docs/rules README.md
git commit -m "fix(suppress): leer los ignores con el scanner de postgres y avisar de reglas desconocidas"
```

---

### Task 7: SQL sugerido para `DROP INDEX` y para `add-column-rewrite`

Pendientes que resuelve: #3 y #6 del hito 2.

**Files:** `src/rules/drop_index_not_concurrently.rs`, `src/rules/add_column_rewrite.rs`, `src/catalog.rs`, `tests/rules.rs`

**Interfaces:**
- `catalog.rs` gana `pub(crate) enum RewriteCause { VolatileDefault, Serial, Identity, Generated }` y `pub(crate) fn rewrite_cause(column: &ColumnDef) -> Option<RewriteCause>`. `add_column_cost` pasa a usarla: es Rewrite si y solo si hay causa.
- Help de `add-column-rewrite` según la causa:

| Causa | `help` |
|---|---|
| VolatileDefault | (el actual) `Add the column without a default (or with a constant default), then backfill in batches and set the default afterwards.` |
| Serial / Identity | `Add a plain nullable column, backfill it in batches from a sequence, make it NOT NULL with the set-not-null pattern, then ALTER COLUMN ... ADD GENERATED ... AS IDENTITY (catalog-only).` |
| Generated | `PostgreSQL cannot add a stored generated column without rewriting. Add a regular column kept in sync by the application or a trigger, and backfill it in batches.` |

- `drop-index-not-concurrently`, help:
  - con varios índices ⇒ una línea `DROP INDEX CONCURRENTLY {idx};` por índice, conservando `IF EXISTS`;
  - con `CASCADE` ⇒ se quita `CASCADE` y se agrega la línea `(CONCURRENTLY does not support CASCADE; drop the dependent objects first.)`.

- [ ] **Step 1: Tests que fallan:**
  - `drop_index_help_splits_and_drops_cascade`: `DROP INDEX IF EXISTS a, b CASCADE;` ⇒ help con `DROP INDEX CONCURRENTLY IF EXISTS a;` y `DROP INDEX CONCURRENTLY IF EXISTS b;`, sin `CASCADE` en el SQL y con la nota.
  - `add_column_rewrite_help_by_cause`: tres casos (`gen_random_uuid()`, `GENERATED ALWAYS AS IDENTITY`, `GENERATED ALWAYS AS (a * 2) STORED`) con su help exacto.
  - Ampliar `help_sql_is_valid_sql` con `DROP INDEX IF EXISTS a, b CASCADE;`.
- [ ] Pasos 2–4: `cargo test --test rules` ⇒ FAIL; implementar; `cargo test` ⇒ PASS.
- [ ] **Step 5: Commit**
```bash
git add src tests/rules.rs
git commit -m "fix(rules): sugerir sql válido para drop index múltiple y ayudas según la causa de la reescritura"
```

---

### Task 8: Reglas con varias tablas y `PRIMARY KEY` en línea

Pendientes que resuelve: #4 y #7 del hito 2.

**Files:** `src/rules/mod.rs`, `src/rules/drop_object.rs`, `src/rules/rewrite_table.rs`, `src/rules/add_column_not_null_without_default.rs`, `tests/rules.rs`

**Interfaces:**
- `RuleCtx` gana `pub fn existing_targets(&self) -> Vec<&Effect>`: todos los efectos sin `new_table`, en orden.
- `drop-object` (con `DROP TABLE`) y `rewrite-table` usan `existing_targets()`. El mensaje nombra todas las tablas, cada una entre comillas y separadas por `, ` (por ejemplo `Dropping "a", "b" breaks …`).
- `add-column-not-null-without-default` trata un `PRIMARY KEY` en línea como `NOT NULL`.

- [ ] **Step 1: Tests que fallan:**
  - `drop_table_names_every_existing_table`: `CREATE TABLE new_t (a int); DROP TABLE new_t, users, orders;` ⇒ un `drop-object` cuyo mensaje empieza por `Dropping "users", "orders"`.
  - `vacuum_full_several_tables`: `VACUUM FULL users, orders;` (Autocommit) ⇒ `rewrite-table` nombra ambas.
  - `inline_primary_key_is_not_null`: `ALTER TABLE users ADD COLUMN q int PRIMARY KEY;` ⇒ incluye `add-column-not-null-without-default`.
- [ ] Pasos 2–5 como en la Task 7. Commit:
```bash
git add src/rules tests/rules.rs
git commit -m "fix(rules): revisar todas las tablas de una sentencia y tratar primary key en línea como not null"
```

---

### Task 9: Detalles de salida

Pendientes que resuelve: #5, #9 y #10 del hito 2.

**Files:** `src/config.rs`, `src/report/json.rs`, `src/analyze.rs`, `tests/config.rs`, `tests/json.rs` (+ snapshot), `tests/analyze.rs`

- `ConfigError::Invalid` usa `e.to_string()` de `toml`, que incluye línea, columna y la clave.
- En JSON, los hallazgos de cada sentencia llevan `"line": <línea de la sentencia>`; los de unidad conservan la suya.
- `TransactionSummary.statement_count` no cuenta `VariableSetStmt`.

- [ ] **Step 1: Tests que fallan:**
  - `invalid_value_reports_position`: `[rules]\ndrop-object = "of"\n` ⇒ el mensaje contiene `line 2` y `drop-object`.
  - En `json_is_valid_and_stable`, comprobar que `files[0].statements[0].findings[0].line == 1`, y revisar el snapshot.
  - `set_does_not_count_as_a_statement_in_transaction_summary`: `SET LOCAL lock_timeout = '5s'; ALTER TABLE users ADD COLUMN a int;` ⇒ `transactions` vacío.
- [ ] Pasos 2–4 como en las tareas anteriores (`cargo insta test --review` para el snapshot JSON).
- [ ] **Step 5: Commit**
```bash
git add src tests
git commit -m "fix(report): posición en errores de config, línea en hallazgos json y resumen de transacción sin set"
```

---

## Lo que queda para el hito 4

Adaptador Laravel (`--from laravel`), SARIF, el formato `github`, `--history` (reutiliza `Schema::observe` sin reportar), la GitHub Action, binarios de release y el GIF del README.
