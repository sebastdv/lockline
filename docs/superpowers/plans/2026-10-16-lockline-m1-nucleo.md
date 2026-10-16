# lockline — Hito 1 (núcleo) · Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Una CLI `lockline check` que parsea migraciones `.sql` con el parser real de Postgres, informa el lock y el costo de cada sentencia de índices y `ADD COLUMN`, detecta tres patrones peligrosos y falla en CI con el código de salida correcto.

**Architecture:** Crate único `lib` + `bin`. `parse` (pg_query → sentencias con posición) → `catalog` (sentencia → efectos: tabla, lock, costo) → `analyze` (recorre cada unidad con un `UnitContext` de tablas nuevas y transacción, y corre las reglas) → `Report` → `report::human`. `main.rs` solo traduce flags y decide el código de salida.

**Tech Stack:** Rust estable (edición 2024), `pg_query` 6.2, `clap` 4 (derive), `thiserror` 2, `owo-colors` 4; tests con `insta` 1, `assert_cmd` 2, `predicates` 3, `tempfile` 3.

**Spec:** `docs/superpowers/specs/2026-10-16-lockline-design.md` (este plan cubre su §10 hito 1; los hitos 2–4 tendrán su propio plan).

## Global Constraints

- Rust estable, `edition = "2024"`; `cargo fmt --check` y `cargo clippy --all-targets -- -D warnings` limpios al cerrar cada tarea.
- Licencia `MIT OR Apache-2.0`; nombre del crate y del binario: `lockline`.
- Código, identificadores, mensajes de la CLI y README en **inglés**.
- IDs de reglas en kebab-case, exactos: `create-index-not-concurrently`, `cannot-run-in-transaction`, `add-column-rewrite`, `parse-error`.
- Códigos de salida: `0` sin hallazgos ≥ umbral, `1` hallazgos ≥ umbral (`--fail-on`, por defecto `error`), `2` error de uso/IO.
- Ninguna regla (salvo `cannot-run-in-transaction`) dispara sobre una tabla creada en la misma unidad de migración.
- Nunca inventar un lock: sentencia fuera del catálogo ⇒ `not analyzed`.
- Sin código, datos ni nombres de Sódeker en fixtures.
- Commits: Conventional Commits en español, una sola línea `tipo(scope): descripción`, **sin cuerpo y sin `Co-Authored-By`**. Salvo que el usuario diga "haz los commits", el ejecutor le entrega los comandos `git add`/`git commit` para copiar en vez de ejecutarlos.

## Review Focus

1. **Finales de línea CRLF** (migraciones editadas en Windows) → los números de línea siguen siendo correctos. Test en Task 2.
2. **Texto UTF-8 multibyte antes de una sentencia** (comentarios con tildes, `ñ`) → la columna cuenta caracteres, no bytes, y recortar el texto no hace panic. Test en Task 2.
3. **Identificadores con comillas y con esquema** (`"Users"` ≠ `users`; `public.users` = `users`) → la detección de tabla nueva coincide como en Postgres. Test en Task 5.
4. **Archivo vacío o solo con comentarios** → 0 sentencias, sin error, cuenta como archivo analizado. Tests en Task 2 y Task 8.
5. **Archivo con BOM UTF-8 y directorios anidados con archivos que no son `.sql`** (estructura de Prisma `migrations/<ts>_x/migration.sql`) → se quita el BOM, solo se leen `.sql`, en orden determinista. Test en Task 6.

---

### Task 1: Toolchain y esqueleto del crate

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `src/main.rs`, `.gitignore`, `LICENSE-MIT`, `LICENSE-APACHE`, `tests/smoke.rs`
- Add to commit: `docs/` (diseño y este plan)

**Interfaces:**
- Produces: crate `lockline` que compila con `pg_query` enlazado.

- [ ] **Step 1: Instalar Rust (requiere aprobación del usuario: modifica su perfil de shell)**

Run: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env" && rustc --version && cargo install cargo-insta`
Expected: `rustc 1.9x.x` (≥ 1.85 para edición 2024) y `cargo-insta` instalado.

- [ ] **Step 2: Crear el crate**

`cargo init --name lockline` en la raíz del repo. `Cargo.toml`: `edition = "2024"`, `license = "MIT OR Apache-2.0"`, `description = "Explains the locks your PostgreSQL migrations take and flags the dangerous ones"`, `repository` vacío por ahora. Dependencias: `pg_query = "6.2"`, `clap = { version = "4", features = ["derive"] }`, `thiserror = "2"`, `owo-colors = "4"`. Dev: `insta = "1"`, `assert_cmd = "2"`, `predicates = "3"`, `tempfile = "3"`. Textos de licencia estándar con `Copyright (c) 2026` y el nombre del autor (preguntar cómo quiere figurar si no está claro). `.gitignore`: `/target`.

- [ ] **Step 3: Escribir el smoke test**

```rust
// tests/smoke.rs
#[test]
fn pg_query_parses_postgres_sql() {
    let result = pg_query::parse("CREATE INDEX CONCURRENTLY i ON t (c);").unwrap();
    assert_eq!(result.protobuf.stmts.len(), 1);
}
```

- [ ] **Step 4: Compilar y correr**

Run: `cargo test --test smoke`
Expected: PASS. La primera compilación de `pg_query` compila C (tarda 1–3 min). Si falla por `libclang`, correr `xcode-select --install`; `protoc` **no** es necesario.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src .gitignore LICENSE-MIT LICENSE-APACHE tests docs
git commit -m "build(core): crear crate lockline con pg_query y documentos de diseño"
```

---

### Task 2: Parser con posiciones

**Files:**
- Create: `src/parse.rs`
- Modify: `src/lib.rs` (`pub mod parse;`)
- Test: `src/parse.rs` (módulo `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct Statement { pub node: pg_query::NodeEnum, pub text: String, pub line: usize, pub col: usize }` — `line`/`col` 1-based, del primer carácter significativo de la sentencia (no de los espacios ni comentarios previos); `col` en **caracteres**; `text` sin espacios/comentarios iniciales ni `;` final.
  - `pub struct ParseError { pub message: String }` (+ `Display`, `std::error::Error` vía `thiserror`).
  - `pub fn parse_sql(sql: &str) -> Result<Vec<Statement>, ParseError>`

- [ ] **Step 1: Tests que fallan**

```rust
#[test] fn parses_two_statements_with_positions() {
    let sql = "-- add index\nCREATE INDEX i ON t (c);\n\n  ALTER TABLE t ADD COLUMN x int;\n";
    let s = parse_sql(sql).unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!((s[0].line, s[0].col), (2, 1));
    assert_eq!(s[0].text, "CREATE INDEX i ON t (c)");
    assert_eq!((s[1].line, s[1].col), (4, 3));
    assert_eq!(s[1].text, "ALTER TABLE t ADD COLUMN x int");
}
#[test] fn crlf_line_numbers() {
    let s = parse_sql("SELECT 1;\r\n\r\nSELECT 2;\r\n").unwrap();
    assert_eq!(s[1].line, 3);
}
#[test] fn multibyte_text_counts_chars_not_bytes() {
    let s = parse_sql("SELECT 'ñandú'; /* acción */ SELECT 2;").unwrap();
    assert_eq!((s[1].line, s[1].col), (1, 30));
}
#[test] fn last_statement_without_semicolon() {
    let s = parse_sql("SELECT 1;\nSELECT 2").unwrap();
    assert_eq!(s[1].text, "SELECT 2");
}
#[test] fn empty_and_comment_only_inputs_have_no_statements() {
    assert!(parse_sql("").unwrap().is_empty());
    assert!(parse_sql("-- nothing here\n/* still nothing */\n").unwrap().is_empty());
}
#[test] fn syntax_error_is_reported() {
    let e = parse_sql("CREATE INDEX ON;").unwrap_err();
    assert!(!e.message.is_empty());
}
```

- [ ] **Step 2: Verificar que fallan**

Run: `cargo test parse::`
Expected: FAIL (no compila: `parse_sql` no existe).

- [ ] **Step 3: Implementar `parse_sql`**

Usar `pg_query::parse(sql)` y recorrer `result.protobuf.stmts`. Cada `RawStmt` trae `stmt_location` (byte offset) y `stmt_len` (`0` = hasta el final). El rango incluye los espacios y comentarios que siguen al `;` anterior: avanzar el inicio saltando espacios en blanco, comentarios `-- …\n` y `/* … */`, y quitar del final espacios y el `;`. Calcular línea y columna sobre `sql[..inicio]` contando `\n` y luego `chars()` desde el último `\n` (un `\r` antes del `\n` no suma columna porque se reinicia en cada `\n`). Todos los cortes van sobre límites de carácter (los offsets de pg_query lo son).

- [ ] **Step 4: Verificar que pasan**

Run: `cargo test parse::`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add src/parse.rs src/lib.rs
git commit -m "feat(parse): parsear sentencias sql con línea y columna"
```

---

### Task 3: Modelo de locks y tipos compartidos

**Files:**
- Create: `src/locks.rs`, `src/model.rs`
- Modify: `src/lib.rs`
- Test: módulos `#[cfg(test)]` en ambos archivos

**Interfaces:**
- Produces (`src/locks.rs`):
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)] pub enum LockMode { AccessShare, RowShare, RowExclusive, ShareUpdateExclusive, Share, ShareRowExclusive, Exclusive, AccessExclusive }` (en ese orden, de menor a mayor).
  - `impl LockMode { pub fn conflicts_with(self, other: LockMode) -> bool; pub fn blocks_reads(self) -> bool; pub fn blocks_writes(self) -> bool; pub fn as_str(self) -> &'static str }` — `as_str` en mayúsculas con espacios: `"ACCESS EXCLUSIVE"`.
- Produces (`src/model.rs`):
  - `pub struct TableRef { pub schema: Option<String>, pub name: String }` con `pub fn from_range_var(rv: &pg_query::protobuf::RangeVar) -> TableRef` (esquema vacío ⇒ `None`). `PartialEq`/`Eq`/`Hash` manuales sobre la clave `(schema.unwrap_or("public"), name)`. `Display`: `schema.name` si hay esquema, si no `name`.
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] pub enum Cost { Metadata, Scan, Rewrite, Unknown }`
  - `pub struct Effect { pub table: TableRef, pub lock: LockMode, pub cost: Cost, pub note: Option<String> }`
  - `pub enum Analysis { Locks(Vec<Effect>), NoTableLock, NotAnalyzed }`
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] pub enum Severity { Warning, Error }`
  - `pub struct Finding { pub rule: &'static str, pub severity: Severity, pub message: String, pub help: Option<String> }`

- [ ] **Step 1: Tests que fallan**

Matriz de conflictos de la documentación de Postgres ("Explicit Locking"), escrita completa en el test `conflict_matrix_matches_postgres_docs` como lista de pares que **sí** conflictan; todo par no listado debe ser `false`:

| Modo | Conflicta con |
|---|---|
| AccessShare | AccessExclusive |
| RowShare | Exclusive, AccessExclusive |
| RowExclusive | Share, ShareRowExclusive, Exclusive, AccessExclusive |
| ShareUpdateExclusive | ShareUpdateExclusive, Share, ShareRowExclusive, Exclusive, AccessExclusive |
| Share | RowExclusive, ShareUpdateExclusive, ShareRowExclusive, Exclusive, AccessExclusive |
| ShareRowExclusive | RowExclusive, ShareUpdateExclusive, Share, ShareRowExclusive, Exclusive, AccessExclusive |
| Exclusive | RowShare, RowExclusive, ShareUpdateExclusive, Share, ShareRowExclusive, Exclusive, AccessExclusive |
| AccessExclusive | todos |

Más:
- `conflicts_are_symmetric`: para los 64 pares, `a.conflicts_with(b) == b.conflicts_with(a)`.
- `only_access_exclusive_blocks_reads`: `blocks_reads()` es `true` solo para `AccessExclusive`.
- `blocks_writes_set`: `true` exactamente para `Share`, `ShareRowExclusive`, `Exclusive`, `AccessExclusive`.
- `table_ref_public_schema_equals_unqualified`: `TableRef{Some("public"),"users"} == TableRef{None,"users"}` y tienen el mismo hash; `TableRef{None,"Users"} != TableRef{None,"users"}`.

- [ ] **Step 2: Verificar que fallan** — Run: `cargo test locks:: model::` · Expected: FAIL (no compila).

- [ ] **Step 3: Implementar** `LockMode` con la tabla anterior como `match`, `blocks_reads = conflicts_with(AccessShare)`, `blocks_writes = conflicts_with(RowExclusive)`; y los tipos de `model.rs`.

- [ ] **Step 4: Verificar que pasan** — Run: `cargo test locks:: model::` · Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/locks.rs src/model.rs src/lib.rs
git commit -m "feat(locks): modelar modos de lock de postgres y su matriz de conflictos"
```

---

### Task 4: Catálogo (índices, `ADD COLUMN`, sentencias sin transacción) y volatilidad

**Files:**
- Create: `src/volatility.rs`, `src/catalog.rs`
- Modify: `src/lib.rs`
- Test: `tests/catalog.rs` (usa `parse_sql` para construir los nodos)

**Interfaces:**
- Consumes: `parse::parse_sql`, `model::{Analysis, Effect, Cost, TableRef}`, `locks::LockMode`.
- Produces:
  - `pub enum Volatility { Safe, Volatile, Unknown }` y `pub fn classify_default(expr: &pg_query::protobuf::Node) -> Volatility` (`src/volatility.rs`).
  - `pub fn analyze_statement(node: &pg_query::NodeEnum) -> Analysis` (`src/catalog.rs`).
  - `pub fn forbidden_in_transaction(node: &pg_query::NodeEnum) -> Option<&'static str>`: devuelve la etiqueta a mostrar (`"CREATE INDEX CONCURRENTLY"`, `"DROP INDEX CONCURRENTLY"`, `"REINDEX CONCURRENTLY"`, `"DETACH PARTITION CONCURRENTLY"`, `"VACUUM"`) o `None`.

Alcance de `analyze_statement` en este hito (el resto del catálogo del spec §5 es del hito 2):

| Nodo | Resultado |
|---|---|
| `IndexStmt` sin `concurrent` | `Locks([Effect{table, Share, Scan, note: Some("index build")}])` |
| `IndexStmt` con `concurrent` | `Locks([Effect{table, ShareUpdateExclusive, Scan, note: Some("index build, does not block writes")}])` |
| `AlterTableStmt` donde **todos** los subcomandos son `AtAddColumn` | `Locks([Effect{table, AccessExclusive, max(costos), None}])`; costo por columna abajo |
| `CreateStmt`, `TransactionStmt`, `VariableSetStmt`, `CreateFunctionStmt`, `ViewStmt`, `CommentStmt`, `CreateEnumStmt` | `NoTableLock` |
| cualquier otro (incl. `AlterTableStmt` con otros subcomandos) | `NotAnalyzed` |

Costo de `ADD COLUMN`: `Rewrite` si el tipo es `serial`/`bigserial`/`smallserial`/`serial2`/`serial4`/`serial8`, si tiene constraint `IDENTITY` o `GENERATED … STORED`, o si `classify_default` del `DEFAULT` da `Volatile`; si no, `Metadata`.

`classify_default`: `Volatile` si hay un `FuncCall` cuyo último nombre está en `random`, `gen_random_uuid`, `uuid_generate_v1`, `uuid_generate_v1mc`, `uuid_generate_v4`, `clock_timestamp`, `timeofday`, `nextval`, `txid_current`, `pg_current_xact_id`. `Safe` para `A_Const`, `SQLValueFunction` (`CURRENT_TIMESTAMP`, `CURRENT_DATE`…), `FuncCall` en `now`, `transaction_timestamp`, `statement_timestamp`, `TypeCast` de algo `Safe`, y `A_Expr`/`A_ArrayExpr`/`BoolExpr` cuyos hijos son todos `Safe`. Cualquier otra cosa ⇒ `Unknown` (el `Unknown` se trata como `Metadata` en este hito; su regla de warning es del hito 2). Documentar las dos listas en un comentario de cabecera de `volatility.rs`.

- [ ] **Step 1: Tests que fallan** (`tests/catalog.rs`; helper `fn analyze(sql) -> Analysis` que parsea una sentencia y llama a `analyze_statement`)

- `create_index_takes_share_and_scans`: `CREATE INDEX i ON users (email)` ⇒ un efecto, tabla `users`, `Share`, `Scan`.
- `create_index_concurrently_takes_share_update_exclusive`: ⇒ `ShareUpdateExclusive`.
- `add_column_without_default_is_metadata`: `ALTER TABLE users ADD COLUMN nick text` ⇒ `AccessExclusive`, `Metadata`.
- `add_column_with_constant_default_is_metadata`: `... ADD COLUMN active boolean DEFAULT true` ⇒ `Metadata`.
- `add_column_with_now_default_is_metadata`: `... DEFAULT now()` y `... DEFAULT CURRENT_TIMESTAMP` ⇒ `Metadata`.
- `add_column_with_volatile_default_rewrites`: `... ADD COLUMN token uuid DEFAULT gen_random_uuid()` ⇒ `Rewrite`.
- `add_serial_or_identity_column_rewrites`: `ADD COLUMN n bigserial` y `ADD COLUMN n int GENERATED ALWAYS AS IDENTITY` ⇒ `Rewrite`.
- `add_stored_generated_column_rewrites`: `ADD COLUMN t int GENERATED ALWAYS AS (a + 1) STORED` ⇒ `Rewrite`.
- `schema_qualified_table_is_kept`: `CREATE INDEX i ON app.users (x)` ⇒ `TableRef{Some("app"),"users"}`.
- `create_table_has_no_table_lock`, `set_has_no_table_lock` (`SET lock_timeout = '2s'`), `begin_has_no_table_lock`.
- `drop_column_is_not_analyzed_yet`: `ALTER TABLE users DROP COLUMN x` ⇒ `NotAnalyzed`.
- `forbidden_in_transaction_labels`: `CREATE INDEX CONCURRENTLY i ON t (c)`, `DROP INDEX CONCURRENTLY i`, `REINDEX INDEX CONCURRENTLY i`, `ALTER TABLE p DETACH PARTITION c CONCURRENTLY`, `VACUUM t` y `VACUUM` devuelven su etiqueta exacta; `CREATE INDEX i ON t (c)`, `ANALYZE t` y `SELECT 1` devuelven `None`.

- [ ] **Step 2: Verificar que fallan** — Run: `cargo test --test catalog` · Expected: FAIL (no compila).

- [ ] **Step 3: Implementar** `volatility.rs` y `catalog.rs` según las tablas. Para `VACUUM` vs `ANALYZE`: `VacuumStmt.is_vacuumcmd`. `REINDEX CONCURRENTLY` y `DROP INDEX CONCURRENTLY`: `ReindexStmt.params` contiene un `DefElem` `concurrently`; `DropStmt.concurrent`. `DETACH PARTITION CONCURRENTLY`: subcomando `AtDetachPartition` con `PartitionCmd.concurrent`.

- [ ] **Step 4: Verificar que pasan** — Run: `cargo test --test catalog` · Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/volatility.rs src/catalog.rs src/lib.rs tests/catalog.rs
git commit -m "feat(catalog): catalogar locks de índices y add column con detección de defaults volátiles"
```

---

### Task 5: Análisis por unidad, contexto y reglas

**Files:**
- Create: `src/context.rs`, `src/analyze.rs`, `src/rules/mod.rs`, `src/rules/create_index_not_concurrently.rs`, `src/rules/cannot_run_in_transaction.rs`, `src/rules/add_column_rewrite.rs`
- Modify: `src/lib.rs` (re-exportar `analyze`, `MigrationUnit`, `Config`, `TransactionMode`, `Report` y tipos de reporte)
- Test: `tests/analyze.rs`

**Interfaces:**
- Consumes: `parse::{parse_sql, Statement, ParseError}`, `catalog::{analyze_statement, forbidden_in_transaction}`, `model::*`.
- Produces:
  - `pub struct MigrationUnit { pub name: String, pub sql: String }` — `name` es la ruta tal como se muestra (o `"<stdin>"`).
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)] pub enum TransactionMode { #[default] PerUnit, None }`
  - `#[derive(Default)] pub struct Config { pub transaction: TransactionMode }` (crece en el hito 2).
  - `pub struct StatementReport { pub line: usize, pub col: usize, pub text: String, pub analysis: Analysis, pub new_table: bool, pub findings: Vec<Finding> }` — `new_table` = todas las tablas de sus efectos fueron creadas antes en la unidad.
  - `pub struct UnitReport { pub name: String, pub statements: Vec<StatementReport>, pub findings: Vec<Finding> }` — `findings` de unidad (aquí va `parse-error`).
  - `pub struct Report { pub units: Vec<UnitReport> }` con `pub fn counts(&self) -> Counts` y `pub struct Counts { pub errors: usize, pub warnings: usize, pub files: usize, pub statements: usize }`, y `pub fn max_severity(&self) -> Option<Severity>`.
  - `pub fn analyze(units: &[MigrationUnit], config: &Config) -> Report`
  - `src/context.rs`: `pub struct UnitContext { pub created_tables: HashSet<TableRef>, pub in_transaction: bool }` con `pub fn new(mode: TransactionMode) -> Self` y `pub fn observe(&mut self, node: &pg_query::NodeEnum)` (actualiza después de evaluar las reglas de la sentencia).
  - `src/rules/mod.rs`: `pub struct RuleCtx<'a> { pub stmt: &'a Statement, pub analysis: &'a Analysis, pub new_table: bool, pub in_transaction: bool }`, `pub trait Rule { fn id(&self) -> &'static str; fn severity(&self) -> Severity; fn check(&self, ctx: &RuleCtx) -> Option<Finding>; }`, `pub fn all() -> Vec<Box<dyn Rule>>`.

Reglas de transacción en `observe`: `CreateStmt` agrega la tabla a `created_tables`. `TransactionStmt` `BEGIN`/`START` ⇒ `in_transaction = true`; `COMMIT`/`END`/`ROLLBACK` ⇒ `false`. Estado inicial: `true` si `PerUnit`, `false` si `None`.

Textos exactos de las reglas (el implementador puede pulir la redacción, pero conservar los datos):

| Regla | Severidad | `message` | `help` |
|---|---|---|---|
| `create-index-not-concurrently` | Error | `Building an index without CONCURRENTLY blocks all writes to "{table}" until the build finishes.` | el `CREATE INDEX` original con `CONCURRENTLY` (deparsear un clon con `concurrent = true` vía `NodeEnum::deparse`; si esa API no existe en la versión instalada, insertar `CONCURRENTLY ` después de `INDEX ` en el texto original), seguido de una línea `(CONCURRENTLY cannot run inside a transaction block)` |
| `cannot-run-in-transaction` | Error | `{label} cannot run inside a transaction block; this migration will fail.` | `Run it outside a transaction. In Laravel, add "public $withinTransaction = false;" to the migration class.` |
| `add-column-rewrite` | Error | `Adding this column rewrites "{table}" and all its indexes while holding an ACCESS EXCLUSIVE lock.` | `Add the column without a default (or with a constant default), then backfill in batches and set the default afterwards.` |
| `parse-error` | Error | `Could not parse this file: {message}` | — |

Condiciones: `create-index-not-concurrently` ⇒ `IndexStmt` sin `concurrent` y `!new_table`. `cannot-run-in-transaction` ⇒ `forbidden_in_transaction(...)` es `Some` y `in_transaction` (sin importar `new_table`). `add-column-rewrite` ⇒ efecto con `Cost::Rewrite` sobre `AlterTableStmt` y `!new_table`.

- [ ] **Step 1: Tests que fallan** (`tests/analyze.rs`, helper `fn run(sql, mode) -> UnitReport`)

- `index_on_existing_table_is_an_error`: `CREATE INDEX i ON users (email);` ⇒ un finding `create-index-not-concurrently`, `Error`, `help` contiene `CREATE INDEX CONCURRENTLY i ON users`.
- `index_on_table_created_in_same_unit_is_fine`: `CREATE TABLE users (id int); CREATE INDEX i ON users (id);` ⇒ cero findings y el `StatementReport` del índice tiene `new_table == true`.
- `quoted_and_schema_names_match_like_postgres`: `CREATE TABLE public.users (id int); CREATE INDEX a ON users (id);` ⇒ sin findings; `CREATE TABLE "Users" (id int); CREATE INDEX b ON users (id);` ⇒ un finding (son tablas distintas).
- `concurrently_in_per_unit_mode_fails`: `CREATE INDEX CONCURRENTLY i ON users (email);` con `PerUnit` ⇒ finding `cannot-run-in-transaction` con mensaje que empieza por `CREATE INDEX CONCURRENTLY cannot run`; con `TransactionMode::None` ⇒ cero findings.
- `explicit_commit_ends_transaction`: `BEGIN; ALTER TABLE users ADD COLUMN x int; COMMIT; CREATE INDEX CONCURRENTLY i ON users (x);` con `None` ⇒ cero findings; con `None` y sin el `COMMIT` ⇒ un `cannot-run-in-transaction`.
- `concurrently_on_new_table_still_fails_in_transaction`: `CREATE TABLE t (c int); CREATE INDEX CONCURRENTLY i ON t (c);` con `PerUnit` ⇒ un `cannot-run-in-transaction`.
- `volatile_default_on_existing_table_is_an_error`: `ALTER TABLE users ADD COLUMN token uuid DEFAULT gen_random_uuid();` ⇒ `add-column-rewrite`; la misma sobre tabla creada antes en la unidad ⇒ nada.
- `parse_error_is_a_unit_finding`: `CREATE INDEX ON;` ⇒ `statements` vacío, `findings[0].rule == "parse-error"`, `Error`.
- `counts_and_max_severity`: dos unidades (una con un error, otra limpia con 2 sentencias) ⇒ `Counts{errors:1, warnings:0, files:2, statements:3}`, `max_severity() == Some(Error)`; reporte sin hallazgos ⇒ `None`.

- [ ] **Step 2: Verificar que fallan** — Run: `cargo test --test analyze` · Expected: FAIL (no compila).

- [ ] **Step 3: Implementar** `context.rs`, `rules/*`, `analyze.rs`. Orden por sentencia: `analyze_statement` → calcular `new_table` → correr `rules::all()` → `ctx.observe(node)`.

- [ ] **Step 4: Verificar que pasan** — Run: `cargo test --test analyze` · Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/context.rs src/analyze.rs src/rules src/lib.rs tests/analyze.rs
git commit -m "feat(analyze): analizar unidades con contexto de transacción y primeras reglas"
```

---

### Task 6: Entrada desde archivos, directorios y stdin

**Files:**
- Create: `src/input/mod.rs`, `src/input/files.rs`
- Modify: `src/lib.rs`
- Test: `tests/input.rs` (con `tempfile`)

**Interfaces:**
- Consumes: `analyze::MigrationUnit`.
- Produces:
  - `#[derive(Debug, thiserror::Error)] pub enum InputError { #[error("{path}: no such file or directory")] NotFound { path: String }, #[error("{path}: {source}")] Io { path: String, source: std::io::Error } }`
  - `pub fn collect_units(paths: &[std::path::PathBuf], stdin: &mut dyn std::io::Read) -> Result<Vec<MigrationUnit>, InputError>`

Reglas: `-` lee `stdin` (nombre `"<stdin>"`). Un archivo explícito se lee aunque no termine en `.sql`. Un directorio se recorre **recursivamente**, solo archivos con extensión `.sql`, ordenados por ruta completa (orden lexicográfico de bytes) — así `0001_x.sql` < `0002_y.sql` y Prisma `20260101_a/migration.sql` < `20260102_b/migration.sql`. Los paths explícitos conservan el orden en que se pasan. Se quita un BOM UTF-8 inicial (`\u{FEFF}`). El `name` es la ruta tal como se construyó (`dir/sub/file.sql`).

- [ ] **Step 1: Tests que fallan**

- `directory_is_recursive_sorted_and_sql_only`: crear `m/0002_b.sql`, `m/0001_a.sql`, `m/README.md`, `m/2026_x/migration.sql` ⇒ nombres en orden `m/0001_a.sql`, `m/0002_b.sql`, `m/2026_x/migration.sql` (comparar con sufijos relativos al tempdir).
- `explicit_paths_keep_given_order`.
- `dash_reads_stdin`: `collect_units(&["-".into()], &mut "SELECT 1;".as_bytes())` ⇒ un unit `"<stdin>"`, sql `SELECT 1;`.
- `bom_is_stripped`: archivo con bytes `EF BB BF` + `SELECT 1;` ⇒ `sql == "SELECT 1;"`.
- `missing_path_is_not_found`: ⇒ `InputError::NotFound`.

- [ ] **Step 2: Verificar que fallan** — Run: `cargo test --test input` · Expected: FAIL.
- [ ] **Step 3: Implementar** con `std::fs::read_dir` recursivo (sin dependencias nuevas).
- [ ] **Step 4: Verificar que pasan** — Run: `cargo test --test input` · Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add src/input src/lib.rs tests/input.rs
git commit -m "feat(input): leer migraciones desde archivos, directorios y stdin"
```

---

### Task 7: Salida humana

**Files:**
- Create: `src/report/mod.rs`, `src/report/human.rs`
- Modify: `src/lib.rs`
- Test: `tests/human.rs` + `tests/snapshots/` (insta)

**Interfaces:**
- Consumes: `Report`, `Counts`, `StatementReport`, `LockMode::{as_str, blocks_reads, blocks_writes}`.
- Produces: `pub struct HumanOptions { pub color: bool, pub quiet: bool }` y `pub fn render_human(report: &Report, opts: &HumanOptions) -> String`.

Formato (sin color; con color solo se pintan severidad, id de regla y `help`):

```
{unit.name}
  {line}:{col}  {text_resumido}
       lock   {LOCK} on {table} — {bloqueo}[ (new table)]
       cost   {costo}[ ({note})]
  {severity}[{rule}] {message}
       help: {help, cada línea siguiente indentada 13 espacios}

{N} error(s), {M} warning(s) in {F} file(s) ({S} statement(s) analyzed)
```

- `text_resumido`: espacios en blanco colapsados a un espacio; si pasa de 80 caracteres, los primeros 79 + `…`.
- `{bloqueo}`: lee y escribe ⇒ `blocks reads and writes`; solo escrituras ⇒ `blocks writes, allows reads`; ninguno ⇒ `allows reads and writes`.
- `{costo}`: `Metadata` ⇒ `metadata only (fast)`; `Scan` ⇒ `full table scan`; `Rewrite` ⇒ `rewrites the table and its indexes`; `Unknown` ⇒ `depends on the data`.
- `Analysis::NotAnalyzed` ⇒ una sola línea `       lock   not analyzed`; `NoTableLock` ⇒ sin líneas de lock/costo. Varios efectos ⇒ un par lock/costo por efecto.
- Findings de unidad (parse-error) se imprimen bajo el nombre de la unidad, antes de las sentencias.
- `quiet`: solo se imprimen las sentencias con findings y las unidades que tienen alguno; el resumen siempre.
- Singular/plural: `1 error`, `2 errors`, `1 file`, `1 statement`.
- Una línea en blanco entre unidades.

- [ ] **Step 1: Tests que fallan** (snapshots con `insta::assert_snapshot!`, construyendo el `Report` vía `analyze` + `MigrationUnit`)

- `renders_index_error_and_safe_column`: unidad `migrations/0007_add_email_index.sql` con `CREATE INDEX idx_users_email ON users (email);\nALTER TABLE users ADD COLUMN nickname text;` — revisar a mano que el snapshot coincide con el ejemplo del spec §4 (adaptado al formato de arriba) antes de aceptarlo.
- `renders_new_table_note_and_not_analyzed`.
- `quiet_hides_clean_statements`.
- `pluralization_in_summary`: `0 errors, 0 warnings in 1 file (1 statement analyzed)`.
- `long_statement_is_truncated` (sentencia de > 80 caracteres en varias líneas).
- `parse_error_rendered_under_unit`.

- [ ] **Step 2: Verificar que fallan** — Run: `cargo test --test human` · Expected: FAIL.
- [ ] **Step 3: Implementar** `render_human` (color con `owo-colors`, solo si `opts.color`).
- [ ] **Step 4: Aceptar y verificar** — Run: `cargo insta test --test human --review` (revisar cada snapshot) y luego `cargo test --test human` · Expected: PASS.
- [ ] **Step 5: Commit**

```bash
git add src/report src/lib.rs tests/human.rs tests/snapshots
git commit -m "feat(report): renderizar el reporte humano con lock y costo por sentencia"
```

---

### Task 8: CLI `lockline check` y códigos de salida

**Files:**
- Modify: `src/main.rs`
- Create: `tests/cli.rs`, `tests/fixtures/clean/0001_create_users.sql`, `tests/fixtures/clean/0002_add_column.sql`, `tests/fixtures/dangerous/0001_index.sql`, `tests/fixtures/empty.sql`

**Interfaces:**
- Consumes: `collect_units`, `analyze`, `Config`, `TransactionMode`, `render_human`, `HumanOptions`, `Report::max_severity`.
- Produces: binario `lockline` con subcomando `check`:
  `lockline check [PATHS]... [--quiet] [--fail-on <error|warning>] [--transaction <per-unit|none>] [--color <auto|always|never>]`
  - Sin `PATHS` ⇒ error de uso (exit 2).
  - `--color auto` ⇒ color solo si stdout es TTY (`std::io::IsTerminal`) y `NO_COLOR` no está definido.
  - Imprime `render_human` en stdout; errores de entrada en stderr con prefijo `error: `.
  - Exit: `2` si `InputError` o error de clap; `1` si `max_severity() >= fail_on`; si no, `0`.

- [ ] **Step 1: Tests que fallan** (`assert_cmd::Command::cargo_bin("lockline")`)

- `clean_directory_exits_zero`: `check tests/fixtures/clean` ⇒ exit 0, stdout contiene `0 errors, 0 warnings in 2 files`.
- `dangerous_file_exits_one`: `check tests/fixtures/dangerous/0001_index.sql` ⇒ exit 1, stdout contiene `error[create-index-not-concurrently]`.
- `fail_on_warning_threshold`: `--fail-on warning` sobre `clean` ⇒ exit 0 y sobre `dangerous` ⇒ exit 1 (en este hito no hay reglas de warning; el caso "solo warnings" se prueba en el hito 2).
- `stdin_input`: `check -` con stdin `CREATE INDEX i ON t (c);` ⇒ exit 1, stdout contiene `<stdin>`.
- `transaction_none_flag`: `check - --transaction none` con stdin `CREATE INDEX CONCURRENTLY i ON t (c);` ⇒ exit 0.
- `missing_file_exits_two`: ⇒ exit 2, stderr contiene `error: ` y `no such file or directory`.
- `no_paths_exits_two`.
- `empty_file_is_fine`: `check tests/fixtures/empty.sql` ⇒ exit 0, `in 1 file (0 statements analyzed)`.
- `never_color_has_no_escape_codes`: con `--color always` el stdout contiene `\x1b[`; con `--color never` no.

- [ ] **Step 2: Verificar que fallan** — Run: `cargo test --test cli` · Expected: FAIL.
- [ ] **Step 3: Implementar** `main.rs` con `clap` derive (`#[derive(Parser)]` + `#[derive(Subcommand)]`), devolviendo `std::process::ExitCode`.
- [ ] **Step 4: Verificar que pasan** — Run: `cargo test` (todo) · Expected: PASS.
- [ ] **Step 5: Probar a mano** — Run: `cargo run -- check tests/fixtures` · Expected: salida legible, exit 1.
- [ ] **Step 6: Commit**

```bash
git add src/main.rs tests/cli.rs tests/fixtures
git commit -m "feat(cli): agregar el comando check con códigos de salida para ci"
```

---

### Task 9: CI y README mínimo

**Files:**
- Create: `.github/workflows/ci.yml`, `README.md`, `README.es.md`, `examples/migrations/0001_create_users.sql`, `examples/migrations/0002_add_email_index.sql`

**Interfaces:**
- Consumes: el binario de Task 8.

- [ ] **Step 1: Workflow** `ci.yml`: en `push` y `pull_request`; jobs en `ubuntu-latest` y `macos-latest`; pasos `actions/checkout`, `dtolnay/rust-toolchain@stable` (con `rustfmt, clippy`), `Swatinem/rust-cache`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Fijar las acciones a versión mayor (`@v4`, etc.).
- [ ] **Step 2: README.md** (inglés): qué problema resuelve en 3 líneas; estado "early development — milestone 1"; instalación desde fuente (`cargo install --path .`); ejemplo con `examples/migrations` y su salida real pegada (generada con `cargo run -- check examples/migrations --color never`); tabla de las 4 reglas actuales; sección "Prior art" que nombra `squawk`, `eugene` y `strong_migrations` y el ángulo de `lockline` (spec §1); licencia. `README.es.md`: resumen corto en español con enlace al inglés.
- [ ] **Step 3: Verificación local completa** — Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` · Expected: todo verde, sin warnings.
- [ ] **Step 4: Commit**

```bash
git add .github README.md README.es.md examples
git commit -m "ci(core): agregar ci de github actions y readme inicial"
```

- [ ] **Step 5: Publicación (la hace el usuario)** — crear el repo público `lockline` en GitHub, `git remote add origin …` y `git push -u origin main`; confirmar que el workflow de CI pasa en ambos sistemas.

---

## Lo que queda para los hitos siguientes (cada uno con su plan)

- **Hito 2:** resto del catálogo (§5) y reglas (§6), `add-column-unknown-default`, `missing-lock-timeout`, locks retenidos hasta el commit, esquema virtual, supresiones, `lockline.toml`, `explain`/`rules`, JSON, `docs/rules/`.
- **Hito 3:** `tests/pg_verify.rs`, `docker-compose.yml`, matriz Postgres 14–17 en CI.
- **Hito 4:** adaptador Laravel, SARIF, `github`, `--history`, GitHub Action, release con binarios, README final con GIF.
