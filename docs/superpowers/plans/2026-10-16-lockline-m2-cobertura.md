# lockline — Hito 2 (cobertura) · Plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Completar el catálogo de locks y las reglas del diseño. Esto incluye el esquema virtual, `lock_timeout`, los locks retenidos hasta el commit, las supresiones, `lockline.toml`, `explain`/`rules` y la salida JSON. También cierra los 12 pendientes de la revisión del hito 1.

**Architecture:** Se mantiene el flujo `parse → catalog → analyze → report`. El hito agrega cinco piezas:
- `schema.rs`: un esquema virtual que se acumula en toda la corrida. Lo consulta `catalog` para resolver índices y reconocer ampliaciones de tipo, y lo consultan las reglas para `set-not-null`.
- `UnitContext`: crece con el estado de `lock_timeout` y los locks retenidos.
- `suppress.rs`: lee los comentarios de supresión.
- `config.rs`: lee `lockline.toml`.
- `report/json.rs`: la salida JSON.

Las reglas nuevas siguen el patrón del trait `Rule` del hito 1, con un archivo por regla.

**Tech Stack:** el del hito 1, más `serde` 1 (derive), `serde_json` 1, `toml` 1 y `globset` 0.4.

**Spec:** `docs/superpowers/specs/2026-10-16-lockline-design.md` (§10, hito 2). Plan anterior: `docs/superpowers/plans/2026-10-16-lockline-m1-nucleo.md`.

## Global Constraints

- Todo lo del hito 1 sigue vigente:
  - Edición 2024; `cargo fmt --check` y `cargo clippy --all-targets -- -D warnings` limpios en cada tarea.
  - Código y mensajes en inglés.
  - Códigos de salida 0/1/2.
  - Nunca inventar un lock: lo que no está modelado sale `NotAnalyzed`.
  - Ninguna regla dispara sobre una tabla creada en la misma unidad, salvo `cannot-run-in-transaction`.
- `rust-version = "1.85"` en `Cargo.toml`.
- IDs de reglas exactos (spec §6): `create-index-not-concurrently`, `drop-index-not-concurrently`, `reindex-not-concurrently`, `cannot-run-in-transaction`, `add-column-rewrite`, `add-column-not-null-without-default`, `add-column-unknown-default`, `alter-column-type`, `set-not-null`, `add-check-without-not-valid`, `add-foreign-key-without-not-valid`, `add-unique-without-index`, `rewrite-table`, `refresh-matview-not-concurrently`, `missing-lock-timeout`, `rename-object`, `drop-object`, `backfill-in-migration`, `parse-error`, `ignore-without-reason`.
- Las severidades por defecto son las de spec §6. `ignore-without-reason` es warning.
- Cuando una regla habla de "la tabla", es `effects[0]`: el catálogo siempre pone primero la relación que la sentencia modifica. Una regla "no dispara sobre tabla nueva" cuando `effects[0].new_table` es verdadero.
- Commits: Conventional Commits en español, una sola línea, sin cuerpo ni `Co-Authored-By`. El ejecutor entrega los comandos al usuario, salvo que este diga "haz los commits".

## Review Focus

1. **Sentencias con varios efectos donde solo una tabla es nueva** (`ALTER TABLE new_orders ADD CONSTRAINT … REFERENCES users`) → no dispara ninguna regla, porque la tabla modificada es nueva, y la salida marca `(new table)` solo en `new_orders`. Test en Task 5.
2. **Migraciones de Laravel reales: muchos `ALTER TABLE` sin `SET lock_timeout`** → `missing-lock-timeout` aparece **una sola vez por archivo** y como warning; no cambia el código de salida con `--fail-on error`. Test en Task 11.
3. **Comentario `-- lockline:ignore` dentro de una sentencia de varias líneas o pegado al final de una línea con SQL** → no suprime la siguiente sentencia por error. Solo cuentan los comentarios de línea completa entre el final de la sentencia anterior y el inicio de la actual. Test en Task 12.
4. **`lockline.toml` con un ID de regla mal escrito o una clave desconocida** → exit 2 con un mensaje que nombra el archivo y la clave, sin ignorarlo en silencio. Test en Task 14.
5. **Esquema virtual con nombres calificados y `RENAME` encadenados** (`public.users` → `ALTER TABLE users RENAME TO accounts` → `ALTER TABLE accounts ALTER COLUMN …`) → el tipo previo se sigue encontrando y la ampliación segura no se marca. Test en Task 4.

---

### Task 1: Modelo: tabla nueva por efecto, identificadores, resumen

Pendientes del hito 1 que resuelve: #9 (identificadores entre comillas), #10 (`Cargo.toml`), #11 (resumen) y la mitad de #12 (tabla nueva por efecto).

**Files:**
- Modify: `Cargo.toml`, `src/model.rs`, `src/catalog.rs`, `src/analyze.rs`, `src/rules/mod.rs`, `src/report/human.rs`, `README.md`
- Test: `src/model.rs` (unit), `tests/analyze.rs`, `tests/human.rs` (+ snapshots), `tests/cli.rs`

**Interfaces:**
- Produces:
  - `Effect` gana `pub new_table: bool`, más `impl Effect { pub fn new(table: TableRef, lock: LockMode, cost: Cost) -> Effect; pub fn with_note(self, note: &str) -> Effect }`. `new` deja `note: None` y `new_table: false`. `catalog.rs` construye todos sus efectos con estas dos funciones.
  - `Finding` gana `pub line: Option<usize>`. Solo se llena en los hallazgos de unidad; `Rule::finding` pone `None`.
  - `Counts` gana `pub suppressed: usize` y `pub not_analyzed: usize`. `not_analyzed` cuenta las sentencias con `Analysis::NotAnalyzed`; `suppressed` vale 0 hasta la Task 12.
  - `analyze` marca `effect.new_table = created_tables.contains(&effect.table)` en cada efecto. `StatementReport.new_table` conserva su significado: hay efectos y todos son nuevos.
  - `Display for TableRef` pone comillas a cada parte que no cumple `^[a-z_][a-z0-9_$]*$` y duplica las `"` internas.
- Resumen humano: `{N} error(s), {M} warning(s)[, {S} suppressed] in {F} file(s) ({T} statement(s)[, {X} not analyzed])`. Los corchetes indican que esa parte solo se muestra cuando es mayor que 0.
- `(new table)` se imprime por efecto, según `effect.new_table`.
- `Cargo.toml`: agregar `rust-version = "1.85"` y quitar la línea `repository = ""`.

- [ ] **Step 1: Tests que fallan**
  - `table_ref_display_quotes_when_needed` (en `model.rs`):

    | Parts | Display |
    |---|---|
    | `(None, "Users")` | `"Users"` |
    | `(Some("app"), "users")` | `app.users` |
    | `(None, "my table")` | `"my table"` |
    | `(None, "a\"b")` | `"a""b"` |
    | `(Some("App"), "x")` | `"App".x` |

  - `counts_not_analyzed` (en `tests/analyze.rs`): `SELECT 1; ALTER TABLE users ADD COLUMN a int;` ⇒ `not_analyzed == 1` y `statements == 2`.
  - `effect_new_table_flag`: `CREATE TABLE users (id int); CREATE INDEX i ON users (id);` ⇒ el efecto del índice tiene `new_table == true`.
  - `pluralization_in_summary` (actualizar): un archivo con `SELECT 1;` ⇒ termina en `0 errors, 0 warnings in 1 file (1 statement, 1 not analyzed)\n`. El caso de dos archivos ⇒ `1 error, 0 warnings in 2 files (3 statements, 2 not analyzed)\n`.
  - `tests/cli.rs`: actualizar `quiet_flag_hides_clean_statements` ⇒ `0 errors, 0 warnings in 2 files (3 statements)\n`. Actualizar `empty_file_is_fine` ⇒ `in 1 file (0 statements)`.
- [ ] **Step 2: Verificar que fallan** — Run: `cargo test --no-fail-fast` · Expected: FAIL en los tests nuevos y en los actualizados.
- [ ] **Step 3: Implementar** según las interfaces anteriores.
- [ ] **Step 4: Revisar los snapshots** — Run: `cargo insta test --review`. Expected: solo cambia la línea de resumen. Aceptar y luego correr `cargo test` (todo en verde).
- [ ] **Step 5: README** — regenerar el bloque de salida de ejemplo con `cargo run -q -- check examples/migrations --color never` y pegarlo tal cual.
- [ ] **Step 6: Commit**
```bash
git add Cargo.toml src tests README.md
git commit -m "refactor(model): marcar tabla nueva por efecto y contar sentencias no analizadas"
```

---

### Task 2: Parser: línea del error y comentario final

Pendientes del hito 1 que resuelve: #5 y #6.

**Files:**
- Modify: `src/parse.rs`, `src/analyze.rs`
- Test: `src/parse.rs` (unit), `tests/analyze.rs`, `tests/human.rs` (snapshot `parse_error_rendered_under_unit`)

**Interfaces:**
- Produces:
  - `Statement` gana `pub end_line: usize`: la línea del último carácter de `text`.
  - `ParseError` gana `pub line: Option<usize>`.
- El texto de cada sentencia termina en su último token que no sea comentario. Para eso se usa `pg_query::scan` sobre el rango de la sentencia y se descartan los tokens finales `SqlComment`/`CComment`.
- Cuando `pg_query::parse` falla, `pg_query::split_with_scanner(sql)` divide el archivo y se parsea cada trozo; la línea del error es la del primer trozo que falla, ubicada con `skip_trivia`. El offset de cada trozo se obtiene con `piece.as_ptr() as usize - sql.as_ptr() as usize`. Si el scanner también falla, `line: None`.
- El hallazgo `parse-error` lleva `line` y su mensaje pasa a ser `Could not parse this file (line {n}): {message}`, o el del hito 1 si no hay línea.

- [ ] **Step 1: Tests que fallan**
  - `trailing_comment_is_not_part_of_last_statement`: `"SELECT 1;\nSELECT 2 -- trailing"` ⇒ `s[1].text == "SELECT 2"`. Con `/* c */` al final, igual.
  - `comment_inside_statement_is_kept`: `"SELECT 1 -- one\n + 1;"` ⇒ `text == "SELECT 1 -- one\n + 1"`.
  - `end_line_of_multiline_statement`: `"SELECT\n  1;\nSELECT 2;"` ⇒ `s[0].end_line == 2` y `s[1].end_line == 3`.
  - `parse_error_reports_line`: `"SELECT 1;\n\nCREATE INDEX ON;\nSELECT 2;"` ⇒ `err.line == Some(3)`.
  - `parse_error_line_after_dollar_quotes`: `"CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql;\nSELEC 2;"` ⇒ `Some(2)`.
  - `parse_error_is_a_unit_finding` (actualizar): `findings[0].line == Some(1)` y el mensaje empieza por `Could not parse this file (line 1): `.
- [ ] **Step 2: Verificar que fallan** — `cargo test --no-fail-fast parse` · FAIL.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Verificar** — `cargo insta test --review` (solo cambia el snapshot del parse-error) y `cargo test` · PASS.
- [ ] **Step 5: Commit**
```bash
git add src/parse.rs src/analyze.rs tests
git commit -m "fix(parse): ubicar la línea del error de sintaxis y excluir comentarios finales"
```

---

### Task 3: Entrada: orden natural, symlinks y carpetas vacías

Pendientes del hito 1 que resuelve: #7, #8 y la otra mitad de #12 (orden natural).

**Files:**
- Modify: `src/input/files.rs`, `src/input/mod.rs`, `src/main.rs`
- Test: `tests/input.rs`, `tests/cli.rs`

**Interfaces:**
- Produces:
  - `pub struct Collected { pub units: Vec<MigrationUnit>, pub warnings: Vec<String> }`
  - `pub fn collect_units(paths: &[PathBuf], stdin: &mut dyn Read) -> Result<Collected, InputError>` (cambia el tipo de retorno; actualizar los tests del hito 1).
  - `pub fn natural_cmp(a: &Path, b: &Path) -> std::cmp::Ordering`. Compara componente por componente; dentro de cada uno separa tramos de dígitos y de no dígitos. Los dígitos se comparan por valor numérico (empate ⇒ gana el más corto) y el resto por bytes.
- Reglas de recorrido:
  - Extensión `.sql` sin distinguir mayúsculas.
  - Los directorios que son symlink no se recorren: se revisa `DirEntry::file_type().is_symlink()`. Los archivos que son symlink sí se leen.
  - Un directorio pasado como argumento que no aporta ningún archivo genera el warning `{dir}: no .sql files found`.
- `main` imprime cada warning en stderr como `warning: {w}`. No cambia el código de salida.

- [ ] **Step 1: Tests que fallan**
  - `flyway_versions_sort_naturally`: `V10__c.sql`, `V2__b.sql`, `V1__a.sql` ⇒ `V1__a`, `V2__b`, `V10__c`.
  - `natural_cmp_cases`: `0002 < 0010`; `a2 < a10`; `abc = abc`; `m/2026_x/migration.sql > m/0002_b.sql`.
  - `uppercase_extension_is_included`: `m/0001.SQL` aparece en las unidades.
  - `symlinked_directory_loop_is_not_followed` (`#[cfg(unix)]`): `m/a.sql` más `m/sub/back -> ..` ⇒ exactamente una unidad.
  - `empty_directory_warns`: un directorio sin `.sql` ⇒ `units` vacío y `warnings == ["{dir}: no .sql files found"]`.
  - `tests/cli.rs` `empty_directory_warns_but_exits_zero`: exit 0, stderr contiene `warning: ` y `no .sql files found`.
- [ ] **Step 2: Verificar que fallan** — `cargo test --test input --test cli` · FAIL.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Verificar** — `cargo test` · PASS.
- [ ] **Step 5: Commit**
```bash
git add src/input src/main.rs tests/input.rs tests/cli.rs
git commit -m "fix(input): ordenar migraciones de forma natural y avisar de carpetas sin sql"
```

---

### Task 4: Esquema virtual

**Files:**
- Create: `src/schema.rs`
- Modify: `src/lib.rs`, `src/analyze.rs`
- Test: `tests/schema.rs`

**Interfaces:**
- Produces (`src/schema.rs`):
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct TypeSpec { pub name: String, pub mods: Vec<i64> }`, con `pub fn from_type_name(t: &pg_query::protobuf::TypeName) -> TypeSpec`.
    - `name` es el último nombre del tipo. El parser normaliza: `varchar`, `numeric`, `text`, `int4`, `varbit`.
    - `mods` son los typmods enteros (`A_Const` con `Integer`).
  - `pub fn is_safe_widening(old: &TypeSpec, new: &TypeSpec) -> bool`. Es `true` en estos casos:
    - Tipos iguales.
    - `varchar(n)` → `varchar(m≥n)`.
    - `varchar(n)` → `varchar` sin mods.
    - `varchar(*)` → `text`.
    - `numeric(p,s)` → `numeric(p2≥p, s)` con la misma `s`.
    - `varbit(n)` → `varbit(m≥n)`.
  - `#[derive(Debug, Clone, Default)] pub struct Schema` (campos privados) con:
    - `pub fn observe(&mut self, node: &pg_query::NodeEnum)`
    - `pub fn column_type(&self, table: &TableRef, column: &str) -> Option<&TypeSpec>`
    - `pub fn has_validated_not_null_check(&self, table: &TableRef, column: &str) -> bool`
    - `pub fn index_table(&self, index: &TableRef) -> Option<&TableRef>`. La clave del índice es `TableRef { schema: <esquema de la tabla>, name: idxname }`.
- `observe` reconoce estas sentencias:
  - `CreateStmt`: columnas y `CHECK` a nivel de tabla.
  - `AlterTableStmt`:
    - `AtAddColumn`, `AtDropColumn`.
    - `AtAlterColumnType`: actualiza el tipo.
    - `AtAddConstraint`: `CHECK (col IS NOT NULL)`, que queda validado salvo que tenga `skip_validation`.
    - `AtValidateConstraint`: marca validado el check con ese nombre.
    - `AtDropConstraint`: lo elimina.
  - `RenameStmt`: tabla (mueve la entrada y sus checks) y columna.
  - `DropStmt` de tablas e índices.
  - `IndexStmt` con nombre: registra índice → tabla.
- El check se reconoce como `NullTest { arg: ColumnRef, nulltesttype: IS_NOT_NULL }`; la columna es el último `String` de `ColumnRef.fields`. Un check sin nombre se guarda con clave vacía, ya validado.
- `analyze` crea un `Schema::default()` por corrida. Después de las reglas de cada sentencia llama a `schema.observe(node)`, en el mismo punto que `ctx.observe`. El esquema se acumula entre unidades.

- [ ] **Step 1: Tests que fallan** (`tests/schema.rs`; el helper `schema_from(sql) -> Schema` parsea y observa cada sentencia)
  - `safe_widening_table`:

    | old → new | esperado |
    |---|---|
    | varchar(10)→varchar(20) | true |
    | varchar(20)→varchar(10) | false |
    | varchar(10)→varchar | true |
    | varchar(10)→text | true |
    | numeric(10,2)→numeric(12,2) | true |
    | numeric(10,2)→numeric(12,3) | false |
    | varbit(4)→varbit(8) | true |
    | int4→int8 | false |
    | text→text | true |

    Los `TypeSpec` salen de `CREATE TABLE t (c <tipo>)` parseado.
  - `tracks_columns_from_create_and_alter`: `CREATE TABLE users (email varchar(100)); ALTER TABLE users ADD COLUMN name text;` ⇒ `column_type(users,"email") == varchar[100]` y `column_type(users,"name") == text[]`.
  - `alter_type_and_drop_column`: tras `ALTER COLUMN email TYPE varchar(200)` ⇒ `[200]`; tras `DROP COLUMN email` ⇒ `None`.
  - `rename_chain_keeps_types` (Review Focus 5): `CREATE TABLE public.users (email varchar(50)); ALTER TABLE users RENAME TO accounts; ALTER TABLE accounts RENAME COLUMN email TO mail;` ⇒ `column_type(accounts,"mail") == varchar[50]` y `column_type(users,"email") == None`.
  - `validated_not_null_check`:
    - `ADD CONSTRAINT c CHECK (email IS NOT NULL) NOT VALID` ⇒ `false`.
    - Luego `VALIDATE CONSTRAINT c` ⇒ `true`.
    - Un `CHECK (email IS NOT NULL)` sin `NOT VALID` ⇒ `true` de inmediato.
    - `CHECK (email <> '')` ⇒ `false`.
  - `index_table_lookup`: `CREATE INDEX i ON app.users (x)` ⇒ `index_table(app.i) == Some(app.users)`; después de `DROP INDEX app.i` ⇒ `None`.
- [ ] **Step 2: Verificar que fallan** — `cargo test --test schema` · FAIL (no compila).
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Verificar** — `cargo test` · PASS.
- [ ] **Step 5: Commit**
```bash
git add src/schema.rs src/lib.rs src/analyze.rs tests/schema.rs
git commit -m "feat(schema): construir un esquema virtual con tipos, checks e índices de la corrida"
```

---

### Task 5: Catálogo completo de `ALTER TABLE`

Pendiente del hito 1 que resuelve: #3 (`objtype`).

**Files:**
- Modify: `src/catalog.rs`, `src/analyze.rs`
- Test: `tests/catalog.rs`, `tests/analyze.rs`

**Interfaces:**
- Consumes: `Schema` (Task 4) y `Effect::new`/`with_note` (Task 1).
- Produces: `pub fn analyze_statement(node: &NodeEnum, schema: &Schema) -> Analysis`. Cambia la firma: actualizar `analyze.rs` y el helper de `tests/catalog.rs`, que usará `&Schema::default()`. Agregar `fn analyze_after(setup: &str, sql: &str) -> Analysis`, que observa `setup` en un esquema antes de analizar.
- Solo se analiza `objtype == ObjectTable`; cualquier otro ⇒ `NotAnalyzed`.
- Todos los subcomandos tienen que estar en la tabla; si alguno falta, toda la sentencia es `NotAnalyzed`.
- El efecto principal (la tabla modificada, `effects[0]`) toma el lock máximo y el costo máximo de los subcomandos. Su nota es la del primer subcomando que alcanzó ese costo.
- Los efectos extra (otras tablas) se agregan después. Si una tabla se repite, queda una sola vez con el lock más fuerte.

| Subcomando | Lock tabla | Costo tabla | Efecto extra |
|---|---|---|---|
| `AtAddColumn` sin restricciones | AccessExclusive | hito 1 (Metadata/Rewrite) | — |
| `AtAddColumn` con `CHECK` / `EXCLUDE` en línea | AccessExclusive | Scan, nota `validates every row` | — |
| `AtAddColumn` con `UNIQUE` / `PRIMARY KEY` en línea | AccessExclusive | Scan, nota `builds an index` | — |
| `AtAddColumn` con `REFERENCES` en línea | AccessExclusive | Scan, nota `validates the foreign key` | `pktable`: ShareRowExclusive, Metadata |
| `AtDropColumn`, `AtDropNotNull`, `AtColumnDefault`, `AtDropConstraint` | AccessExclusive | Metadata | — |
| `AtAlterColumnType` | AccessExclusive | Rewrite; Metadata si no hay `USING` (`ColumnDef.raw_default`) y `is_safe_widening(tipo previo, nuevo)` | — |
| `AtSetNotNull` | AccessExclusive | Scan; Metadata si `has_validated_not_null_check` | — |
| `AtAddConstraint` CHECK | AccessExclusive | Scan; Metadata si `skip_validation` | — |
| `AtAddConstraint` FOREIGN | ShareRowExclusive | Scan; Metadata si `skip_validation` | `pktable`: ShareRowExclusive, Metadata |
| `AtAddConstraint` UNIQUE/PRIMARY | AccessExclusive | Scan, nota `builds an index`; Metadata si `indexname` no está vacío (`USING INDEX`) | — |
| `AtAddConstraint` EXCLUSION | AccessExclusive | Scan | — |
| `AtValidateConstraint` | ShareUpdateExclusive | Scan | — |
| `AtSetStatistics`, `AtSetOptions`, `AtResetOptions`, `AtSetRelOptions`, `AtResetRelOptions` | ShareUpdateExclusive | Metadata | — |
| `AtAttachPartition` | ShareUpdateExclusive | Metadata | partición: AccessExclusive, Scan, nota `validates the partition constraint` |
| `AtDetachPartition` | AccessExclusive (ShareUpdateExclusive si `concurrent`) | Metadata | — |

- [ ] **Step 1: Tests que fallan** (`tests/catalog.rs`)
  - `alter_table_catalog`: cada fila de la tabla anterior con un SQL de ejemplo, comprobando lock, costo y tabla de `effects[0]`. También los efectos extra (`users` con ShareRowExclusive para la FK).
  - `alter_column_type_widening_uses_schema`: `analyze_after("CREATE TABLE users (email varchar(100));", "ALTER TABLE users ALTER COLUMN email TYPE varchar(200)")` ⇒ Metadata. Con `TYPE varchar(50)` ⇒ Rewrite. Sin setup ⇒ Rewrite. Con `USING email::varchar(200)` ⇒ Rewrite.
  - `set_not_null_after_validated_check_is_metadata`.
  - `add_column_with_inline_references_has_two_effects` (reemplaza a `add_column_with_inline_constraints_is_not_analyzed_yet` del hito 1).
  - `non_table_alter_is_not_analyzed`: `ALTER TYPE addr ADD ATTRIBUTE zip text`, `ALTER FOREIGN TABLE ft ADD COLUMN c int` y `ALTER INDEX i SET (fillfactor = 70)` ⇒ `NotAnalyzed`.
  - `unknown_subcommand_is_not_analyzed`: `ALTER TABLE users ENABLE ROW LEVEL SECURITY` ⇒ `NotAnalyzed`.
  - `per_effect_new_table` (`tests/analyze.rs`, Review Focus 1): `CREATE TABLE new_orders (id int, user_id int); ALTER TABLE new_orders ADD CONSTRAINT fk FOREIGN KEY (user_id) REFERENCES users (id);` ⇒ `effects[0].new_table == true`, `effects[1].new_table == false`, `StatementReport.new_table == false` y cero findings.
- [ ] **Step 2: Verificar que fallan** — `cargo test --test catalog --test analyze` · FAIL.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Verificar** — `cargo test` · PASS.
- [ ] **Step 5: Commit**
```bash
git add src/catalog.rs src/analyze.rs tests/catalog.rs tests/analyze.rs
git commit -m "feat(catalog): catalogar todos los subcomandos de alter table con el esquema virtual"
```

---

### Task 6: Catálogo del resto de sentencias

Pendientes del hito 1 que resuelve: #1 (`CONCURRENTLY false`) y #4 (`COMMENT ON`).

**Files:**
- Modify: `src/catalog.rs`
- Test: `tests/catalog.rs`

**Interfaces:**
- Produces: `pub(crate) fn def_elem_bool(d: &pg_query::protobuf::DefElem) -> bool`, con la semántica de `defGetBoolean`:
  - Sin `arg` ⇒ `true`.
  - `Boolean` ⇒ su valor.
  - `Integer` ⇒ distinto de 0.
  - `String`, sin distinguir mayúsculas: `true`/`on`/`yes`/`1` ⇒ `true`; `false`/`off`/`no`/`0` ⇒ `false`.
  - `forbidden_in_transaction` usa esta función para `REINDEX`, y `VACUUM (FULL …)` también.

| Sentencia | Efectos |
|---|---|
| `DropStmt` `ObjectTable` | AccessExclusive, Metadata, uno por tabla |
| `DropStmt` `ObjectIndex` | La tabla sale de `schema.index_table`; AccessExclusive (ShareUpdateExclusive si `concurrent`), Metadata. Si la tabla no se conoce, el efecto va sobre el `TableRef` del índice con la nota `on the index's table (not seen in this run)` |
| `DropStmt` de otro tipo de objeto | `NotAnalyzed` |
| `ReindexStmt` TABLE | Share (ShareUpdateExclusive si concurrently), Scan, nota `rebuilds the table's indexes` |
| `ReindexStmt` INDEX | Tabla resuelta como en `DROP INDEX`; Share/ShareUpdateExclusive, Scan, nota `rebuilds the index` |
| `ReindexStmt` SCHEMA/SYSTEM/DATABASE | `NotAnalyzed` |
| `VacuumStmt` con `FULL` (`def_elem_bool`) | AccessExclusive, Rewrite, por relación |
| `VacuumStmt` sin FULL / `ANALYZE` | ShareUpdateExclusive, Scan, por relación; sin relaciones ⇒ `NotAnalyzed` |
| `ClusterStmt` con relación | AccessExclusive, Rewrite; sin relación ⇒ `NotAnalyzed` |
| `RefreshMatViewStmt` | AccessExclusive (Exclusive si `concurrent`), Rewrite, nota `rebuilds the materialized view` |
| `CreateTrigStmt` | ShareRowExclusive, Metadata |
| `TruncateStmt` | AccessExclusive, Metadata, por relación |
| `InsertStmt` / `UpdateStmt` / `DeleteStmt` | RowExclusive, Unknown |
| `CreateStmt` con `partbound` | padre: AccessExclusive, Metadata |
| `CreateStmt` con `inh_relations` | cada padre: ShareUpdateExclusive, Metadata |
| `CreateStmt` con FK (en columna o de tabla) | cada tabla referenciada: ShareRowExclusive, Metadata |
| `CreateStmt` sin nada de lo anterior | `NoTableLock` (como en el hito 1) |
| `ViewStmt` con `replace` | la vista: AccessExclusive, Metadata, nota `if the view already exists` |
| `CommentStmt` `ObjectTable` / `ObjectColumn` | la tabla (en columnas, todos los nombres menos el último): ShareUpdateExclusive, Metadata |
| `CommentStmt` de otro tipo | `NotAnalyzed` |
| `RenameStmt` `ObjectTable` / `ObjectColumn` / `ObjectTabconstraint` | `relation`: AccessExclusive, Metadata |
| `RenameStmt` de otro tipo | `NotAnalyzed` |

En un `CREATE TABLE` que bloquea otras tablas, `effects[0]` es la primera de esas tablas. Las reglas no miran `CreateStmt`.

- [ ] **Step 1: Tests que fallan**
  - `statement_catalog`: una línea por fila de la tabla, comprobando lock, costo y tablas.
  - `drop_index_resolves_table_from_schema`: con `CREATE INDEX i ON users (x)` previo ⇒ el efecto va sobre `users`. Sin él ⇒ va sobre `i` con la nota indicada.
  - `def_elem_bool_cases`: `REINDEX (CONCURRENTLY false) TABLE t` ⇒ `forbidden_in_transaction == None`; `REINDEX (CONCURRENTLY true) TABLE t` y `REINDEX (CONCURRENTLY) TABLE t` ⇒ `Some("REINDEX CONCURRENTLY")`; `VACUUM (FULL false) t` ⇒ ShareUpdateExclusive, Scan.
  - Actualizar `create_table_that_locks_other_tables_is_not_analyzed_yet` del hito 1, renombrado a `create_table_locks_parent_and_referenced_tables`, con los efectos reales.
- [ ] **Step 2: Verificar que fallan** — `cargo test --test catalog` · FAIL.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Verificar** — `cargo test` · PASS.
- [ ] **Step 5: Commit**
```bash
git add src/catalog.rs tests/catalog.rs
git commit -m "feat(catalog): catalogar drop, reindex, vacuum, cluster, refresh, triggers, dml, comment y rename"
```

---

### Task 7: Contexto: transacciones, `lock_timeout` y locks retenidos

Pendiente del hito 1 que resuelve: #2.

**Files:**
- Modify: `src/context.rs`, `src/analyze.rs`, `src/rules/mod.rs`, `src/report/human.rs`
- Test: `tests/context.rs`, `tests/analyze.rs`, `tests/human.rs`

**Interfaces:**
- Produces (`context.rs`):
  - `UnitContext` gana `pub fn lock_timeout_set(&self) -> bool`. Por dentro distingue el valor de sesión del de `SET LOCAL`, que se borra al terminar la transacción.
  - Reglas de `observe` para `VariableSetStmt` con `name == "lock_timeout"`:
    - `VarSetValue` con valor `0`, `'0'` o `'0ms'` ⇒ sin timeout; cualquier otro valor ⇒ con timeout.
    - `VarSetDefault` y `VarReset` ⇒ sin timeout.
    - `VarResetAll` ⇒ borra el de sesión.
    - `is_local` decide si el valor es de sesión o local.
  - Transacciones:
    - `COMMIT`/`ROLLBACK` con `chain == true` siguen en transacción pero cierran la anterior.
    - `PREPARE TRANSACTION` (`TransStmtPrepare`) cierra la transacción.
- Produces (`analyze.rs`):
  - `pub struct HeldLock { pub table: TableRef, pub lock: LockMode }`
  - `pub struct TransactionSummary { pub ends_after: Option<usize>, pub statement_count: usize, pub locks: Vec<HeldLock> }`. `ends_after` es el índice en `statements` del `COMMIT`/`ROLLBACK`/`PREPARE`; `None` significa el fin implícito de la unidad.
  - `UnitReport.transactions: Vec<TransactionSummary>`. Solo incluye las transacciones con `statement_count ≥ 2` (sin contar `BEGIN`/`COMMIT`) y al menos un lock.
  - Los locks se acumulan por tabla, con el más fuerte, en el orden de la primera adquisición, y excluyen los efectos con `new_table`.
  - `RuleCtx` gana `pub schema: &'a Schema` y `pub lock_timeout_set: bool`.
  - `Rule` gana `fn once_per_unit(&self) -> bool { false }`. Si es `true`, el analizador conserva solo el primer hallazgo de esa regla en la unidad.
- Humano: después del bloque de la sentencia `ends_after` (o al final de la unidad si es `None`) se imprime la línea `  holds until commit: {LOCK} on {table}, {LOCK} on {table}`. No se imprime en `--quiet`.

- [ ] **Step 1: Tests que fallan**
  - `tests/context.rs` `lock_timeout_states`:

    | Secuencia | `lock_timeout_set()` |
    |---|---|
    | (ninguna) | false |
    | `SET lock_timeout = '5s'` | true |
    | `SET lock_timeout = 0` | false |
    | `SET lock_timeout TO DEFAULT` | false |
    | `SET LOCAL lock_timeout = '2s'` (en transacción) | true |
    | `SET LOCAL lock_timeout = '2s'; COMMIT` | false |
    | `SET lock_timeout = '5s'; RESET ALL` | false |

  - `commit_and_chain_keeps_transaction` y `prepare_transaction_ends_it`, comprobando `in_transaction`.
  - `held_locks_summary` (`tests/analyze.rs`):
    - `ALTER TABLE users ADD COLUMN a int; CREATE INDEX i ON orders (x);` (PerUnit) ⇒ un resumen `{ends_after: None, statement_count: 2, locks: [AccessExclusive users, Share orders]}`.
    - Una sola sentencia ⇒ ningún resumen.
    - `BEGIN; ALTER TABLE users ADD COLUMN a int; CREATE INDEX i ON users (a); COMMIT; SELECT 1;` (None) ⇒ `ends_after == Some(3)` y `locks == [AccessExclusive users]`.
    - Una tabla nueva queda excluida.
  - `renders_held_locks` (snapshot en `tests/human.rs`).
- [ ] **Step 2: Verificar que fallan** — `cargo test --test context --test analyze --test human` · FAIL.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Verificar** — `cargo insta test --review` y `cargo test` · PASS.
- [ ] **Step 5: Commit**
```bash
git add src/context.rs src/analyze.rs src/rules/mod.rs src/report/human.rs tests
git commit -m "feat(context): seguir lock_timeout y mostrar los locks retenidos hasta el commit"
```

---

### Tasks 8–11: Reglas nuevas

Patrón común: un archivo por regla en `src/rules/<id_con_guiones_bajos>.rs`, registrado en `rules::all()`. Los tests van en `tests/rules.rs` (se crea en la Task 8), con dos helpers: `fn fired_all(sql: &str, mode: TransactionMode) -> Vec<&'static str>` y `fn fired(...)`. `fired` es igual a `fired_all` pero descarta `missing-lock-timeout`, para que los tests de las Tasks 8–10 no cambien cuando esa regla llegue en la Task 11. Cada regla tiene un test `<regla>_fires` y uno `<regla>_skips`, con los casos de la tabla. Las sentencias `CONCURRENTLY` se prueban con `TransactionMode::None`. El texto de `help` usa el SQL real cuando la tabla lo indica; si hay deparse, se hace con `NodeEnum::deparse` de un clon modificado, como en el hito 1. Cuando una restricción no tiene nombre, el SQL sugerido usa el nombre por defecto de Postgres: `{tabla}_check` para un CHECK y `{tabla}_{primera columna}_fkey` para una FK.

### Task 8: Reglas de índices y mantenimiento

| Regla | Sev. | Dispara | No dispara | `message` | `help` |
|---|---|---|---|---|---|
| `drop-index-not-concurrently` | W | `DROP INDEX i` | `DROP INDEX CONCURRENTLY i`; índice de una tabla nueva | `Dropping an index without CONCURRENTLY takes an ACCESS EXCLUSIVE lock on its table.` | el mismo `DROP INDEX` con `CONCURRENTLY` (deparse) |
| `reindex-not-concurrently` | E | `REINDEX TABLE t`, `REINDEX INDEX i` | `REINDEX TABLE CONCURRENTLY t`; tabla nueva; `REINDEX DATABASE` | `REINDEX without CONCURRENTLY blocks writes to "{table}" while the indexes are rebuilt.` | el mismo `REINDEX` con `CONCURRENTLY` |
| `rewrite-table` | E | `VACUUM FULL t`, `CLUSTER t USING i` | `VACUUM t`; tabla nueva | `{VACUUM FULL\|CLUSTER} rewrites "{table}" under an ACCESS EXCLUSIVE lock; nothing can read or write it until it finishes.` | `Use pg_repack or pg_squeeze to rebuild the table online, outside of migrations.` |
| `refresh-matview-not-concurrently` | W | `REFRESH MATERIALIZED VIEW v` | `… CONCURRENTLY v`; vista creada en la unidad | `Refreshing without CONCURRENTLY blocks every read of "{table}" until it finishes.` | `REFRESH MATERIALIZED VIEW CONCURRENTLY {v}; (requires a unique index on the view)` |
| `backfill-in-migration` | W | `UPDATE users SET a = 1`, `DELETE FROM users` | con `WHERE`; tabla nueva | `Updating every row of "{table}" in a migration holds row locks on the whole table and can run for a long time.` | `Backfill in batches (for example by id ranges) from a separate job, outside the migration transaction.` |

- [ ] **Step 1:** tests `*_fires`/`*_skips` de las 5 reglas · **Step 2:** `cargo test --test rules` ⇒ FAIL · **Step 3:** implementar y registrar · **Step 4:** `cargo test` ⇒ PASS · **Step 5:** commit
```bash
git add src/rules tests/rules.rs
git commit -m "feat(rules): agregar reglas de índices, reindex, reescrituras, refresh y backfill"
```

### Task 9: Reglas de columnas

| Regla | Sev. | Dispara | No dispara | `message` | `help` |
|---|---|---|---|---|---|
| `add-column-not-null-without-default` | E | `ALTER TABLE users ADD COLUMN a int NOT NULL` | con `DEFAULT`; con identity/serial/generated; tabla nueva | `Adding a NOT NULL column without a DEFAULT fails if "{table}" has any rows.` | `Add a DEFAULT, or add the column as nullable, backfill it, then SET NOT NULL.` |
| `add-column-unknown-default` | W | `ADD COLUMN a text DEFAULT my_func()` | default seguro o volátil conocido (este último lo cubre `add-column-rewrite`); tabla nueva | `lockline cannot tell whether the default {expr} is volatile; if it is, "{table}" is rewritten.` | `Use a constant default, or check the function's volatility (pg_proc.provolatile) and add a lockline:ignore comment with the reason.` |
| `alter-column-type` | E | `ALTER COLUMN TYPE` con efecto Rewrite | ampliación segura con tipo previo conocido; tabla nueva | `Changing the type of "{column}" rewrites "{table}" and its indexes under an ACCESS EXCLUSIVE lock.` | si no se conoce el tipo previo, agregar la línea `If this only widens varchar(n) or numeric precision it does not rewrite; lockline did not see the previous type.`; siempre, `Otherwise add a new column, backfill it in batches, and switch reads and writes over.` |
| `set-not-null` | E | `ALTER COLUMN a SET NOT NULL` con efecto Scan | con check validado antes (esquema); tabla nueva | `SET NOT NULL scans all of "{table}" under an ACCESS EXCLUSIVE lock to check for NULLs.` | 4 líneas: `ALTER TABLE {t} ADD CONSTRAINT {t}_{c}_not_null CHECK ({c} IS NOT NULL) NOT VALID;` / `ALTER TABLE {t} VALIDATE CONSTRAINT {t}_{c}_not_null;` / `ALTER TABLE {t} ALTER COLUMN {c} SET NOT NULL;` / `ALTER TABLE {t} DROP CONSTRAINT {t}_{c}_not_null;` |

`{expr}` es el texto de la expresión del default, por ejemplo `my_func()`. Se obtiene así: armar un `SelectStmt` cuyo `target_list` tenga un `ResTarget { val: expr }`, deparsearlo y quitarle el prefijo `SELECT `.

- [ ] Pasos 1–5 como en la Task 8. Commit:
```bash
git add src/rules tests/rules.rs
git commit -m "feat(rules): agregar reglas de columnas not null, defaults desconocidos y cambios de tipo"
```

### Task 10: Reglas de restricciones

| Regla | Sev. | Dispara | No dispara | `message` | `help` |
|---|---|---|---|---|---|
| `add-check-without-not-valid` | E | `ADD CONSTRAINT c CHECK (a > 0)`; `ADD COLUMN` con `CHECK` en línea | con `NOT VALID`; tabla nueva | `Adding a CHECK constraint scans all of "{table}" under an ACCESS EXCLUSIVE lock.` | `{mismo ALTER con NOT VALID};` + `ALTER TABLE {t} VALIDATE CONSTRAINT {c};` (para la variante en línea: agregar la columna primero y luego el constraint) |
| `add-foreign-key-without-not-valid` | E | `ADD CONSTRAINT fk FOREIGN KEY … REFERENCES users`; `ADD COLUMN … REFERENCES` en línea | con `NOT VALID`; la tabla modificada es nueva (aunque la referenciada exista) | `Adding a foreign key scans "{table}" and blocks writes to "{table}" and "{referenced}" until it is validated.` | igual que CHECK, con `NOT VALID` + `VALIDATE CONSTRAINT` |
| `add-unique-without-index` | E | `ADD CONSTRAINT u UNIQUE (email)`, `ADD PRIMARY KEY (id)`, `ADD COLUMN … UNIQUE` en línea | `ADD CONSTRAINT u UNIQUE USING INDEX i`; tabla nueva | `Adding a {UNIQUE\|PRIMARY KEY} constraint builds an index under an ACCESS EXCLUSIVE lock on "{table}".` | `CREATE UNIQUE INDEX CONCURRENTLY {c}_idx ON {t} ({cols});` + `ALTER TABLE {t} ADD CONSTRAINT {c} {UNIQUE\|PRIMARY KEY} USING INDEX {c}_idx;` |

- [ ] Pasos 1–5 como en la Task 8. Commit:
```bash
git add src/rules tests/rules.rs
git commit -m "feat(rules): agregar reglas de check, foreign key y unique sin validación diferida"
```

### Task 11: Compatibilidad y `lock_timeout`

| Regla | Sev. | Dispara | No dispara | `message` | `help` |
|---|---|---|---|---|---|
| `rename-object` | W | `ALTER TABLE users RENAME TO accounts`, `RENAME COLUMN a TO b` | tabla nueva; `RENAME CONSTRAINT` | `Renaming "{name}" breaks application code that still uses the old name while the deploy rolls out.` | `Use expand/contract: add the new name, deploy code that uses it, then remove the old one in a later migration.` |
| `drop-object` | W | `DROP TABLE users`, `ALTER TABLE users DROP COLUMN a` | tabla nueva | `Dropping "{name}" breaks application code that still uses it while the deploy rolls out.` | `Stop using it in the code first, deploy, then drop it in a later migration.` |
| `missing-lock-timeout` | W, `once_per_unit` | primera sentencia de la unidad con un efecto AccessExclusive sobre una tabla no nueva, sin `lock_timeout_set` | con `SET lock_timeout` previo; solo tablas nuevas | `This migration takes an ACCESS EXCLUSIVE lock on "{table}" without a lock_timeout; if the lock has to wait, every query on the table queues behind it.` | `Add SET lock_timeout = '5s'; at the start of the migration (SET LOCAL inside a transaction) and retry on failure.` |

- [ ] **Step 1:** tests `*_fires`/`*_skips`. Además:
  - `missing_lock_timeout_once_per_unit` (Review Focus 2): tres `ALTER TABLE users ADD COLUMN …` ⇒ un solo `missing-lock-timeout`.
  - `cli` `warnings_do_not_fail_by_default`: ese archivo ⇒ exit 0 sin `--fail-on` y exit 1 con `--fail-on warning`.
  - Actualizaciones que trae esta regla, porque toca casi todos los `ALTER TABLE` del hito 1:
    - `tests/fixtures/clean/0002_add_column.sql` empieza con `SET lock_timeout = '5s';`, para que `clean` siga sin warnings.
    - El helper `rules()` de `tests/analyze.rs` descarta `missing-lock-timeout`, que ya tiene sus tests propios.
    - Revisar a mano los snapshots humanos que ahora muestran el warning.
    - `examples/migrations/0002_track_logins.sql` se deja sin `SET`, para que el warning aparezca en la demo.
- [ ] Pasos 2–4 como en la Task 8. Regenerar el ejemplo del README. Commit:
```bash
git add src/rules tests README.md
git commit -m "feat(rules): agregar reglas de renombres, borrados y lock_timeout faltante"
```

---

### Task 12: Supresiones por comentario

**Files:**
- Create: `src/suppress.rs`
- Modify: `src/analyze.rs`, `src/report/human.rs`, `src/lib.rs`
- Test: `tests/suppress.rs`

**Interfaces:**
- Produces:
  - `pub struct Ignore { pub line: usize, pub rules: Vec<String>, pub reason: Option<String>, pub file_wide: bool }`
  - `pub fn parse_ignores(sql: &str) -> Vec<Ignore>`. Solo cuenta un comentario de línea completa, es decir, una línea cuyo `trim_start` empieza por `--`, con la forma `-- lockline:ignore <id>[,<id>…] [motivo…]` o `-- lockline:ignore-file <ids> [motivo…]`. Un motivo vacío ⇒ `None`.
  - `StatementReport` gana `pub suppressed: Vec<Suppressed>`, con `pub struct Suppressed { pub rule: &'static str, pub reason: Option<String> }`.
- Aplicación:
  - A la sentencia `i` le aplican los ignores de línea con `prev.end_line < ignore.line < stmt.line`; para la primera sentencia, `prev.end_line = 0`.
  - También le aplican todos los ignores `file_wide`.
  - Un hallazgo cuya regla está en un ignore aplicable se mueve a `suppressed`.
  - No se pueden suprimir los hallazgos de unidad (`parse-error`).
- `ignore-without-reason` (warning): uno por cada ignore sin motivo.
  - En un ignore de línea, el hallazgo va en la sentencia a la que aplica.
  - En un ignore de archivo, va en la unidad con su `line`.
  - Mensaje: `lockline:ignore for {ids} has no reason; say why this is safe.`
- `Counts.suppressed` cuenta los suprimidos y aparece en el resumen (Task 1).

- [ ] **Step 1: Tests que fallan**
  - `parse_ignores_forms`: línea, archivo, varios IDs y la ausencia de motivo.
  - `ignore_suppresses_next_statement`: `-- lockline:ignore create-index-not-concurrently table is tiny\nCREATE INDEX i ON users (e);` ⇒ cero findings, `suppressed.len() == 1` y `counts().suppressed == 1`.
  - `ignore_does_not_leak_past_statement` (Review Focus 3):
    - `CREATE INDEX a ON t (x);\n-- lockline:ignore create-index-not-concurrently ok\nSELECT 1;\nCREATE INDEX b ON t (y);` ⇒ el índice `b` sigue con su hallazgo.
    - `CREATE INDEX c ON t (z); -- lockline:ignore create-index-not-concurrently ok`, comentario al final de una línea con SQL ⇒ no se suprime.
    - Un comentario dentro de una sentencia de varias líneas no suprime la siguiente.
  - `ignore_file_applies_everywhere`.
  - `ignore_without_reason_warns`: suprime y además produce `ignore-without-reason`.
  - Snapshot `renders_suppressed_summary`: el resumen termina en `, 1 suppressed in 1 file (…)`.
- [ ] **Step 2:** `cargo test --test suppress` ⇒ FAIL · **Step 3:** implementar · **Step 4:** `cargo insta test --review` y `cargo test` ⇒ PASS · **Step 5:** commit
```bash
git add src/suppress.rs src/analyze.rs src/report/human.rs src/lib.rs tests
git commit -m "feat(suppress): suprimir hallazgos con comentarios lockline:ignore con motivo"
```

---

### Task 13: Documentación de reglas, `lockline rules` y `lockline explain`

**Files:**
- Create: `docs/rules/<id>.md` (20 archivos, uno por ID de las Global Constraints)
- Modify: `src/rules/mod.rs`, cada `src/rules/*.rs`, `src/main.rs`
- Test: `tests/docs.rs`, `tests/cli.rs`

**Interfaces:**
- Produces:
  - `Rule` gana `fn summary(&self) -> &'static str` (una línea) y `fn docs(&self) -> &'static str` (`include_str!("../../docs/rules/<id>.md")`).
  - `pub struct RuleInfo { pub id: &'static str, pub severity: Severity, pub summary: &'static str, pub docs: &'static str }`
  - `pub fn infos() -> Vec<RuleInfo>`: las de `all()` en orden de registro, seguidas de `parse-error` e `ignore-without-reason`.
- Formato de cada doc:
````markdown
# <id>

**Severity:** <error|warning>

<qué detecta, 1–2 frases>

## Why it matters

<el lock, qué bloquea y por qué crece con el tamaño de la tabla>

## Instead

```sql
<la alternativa segura>
```

## Ignore it when

<cuándo es razonable suprimirla>
````
- CLI:
  - `lockline rules` imprime una línea por regla con el formato `format!("{:<36} {:<8} {}", id, severity, summary)`.
  - `lockline explain <id>` imprime `docs` tal cual.
  - Un ID desconocido ⇒ stderr `error: unknown rule "<id>"; run "lockline rules" to list them` y exit 2.

- [ ] **Step 1: Tests que fallan**
  - `every_rule_has_docs`: para cada `infos()`, `docs` empieza por `# {id}\n` y contiene `**Severity:** {severity}`, `## Why it matters` e `## Instead`.
  - `infos_cover_all_ids`: el conjunto de IDs es exactamente la lista de las Global Constraints.
  - `cli` `rules_lists_every_rule`, `explain_prints_docs` y `explain_unknown_exits_two`.
- [ ] **Step 2:** `cargo test --test docs --test cli` ⇒ FAIL · **Step 3:** escribir las 20 docs e implementar · **Step 4:** `cargo test` ⇒ PASS · **Step 5:** commit
```bash
git add docs/rules src/rules src/main.rs tests/docs.rs tests/cli.rs
git commit -m "docs(rules): documentar cada regla y agregar los comandos rules y explain"
```

---

### Task 14: `lockline.toml` y `exclude`

**Files:**
- Create: `src/config.rs`
- Modify: `Cargo.toml` (`serde = { version = "1", features = ["derive"] }`, `toml = "1"`, `globset = "0.4"`), `src/analyze.rs`, `src/main.rs`, `src/lib.rs`
- Test: `tests/config.rs`, `tests/cli.rs`

**Interfaces:**
- Produces (`config.rs`):
  - `#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)] #[serde(rename_all = "kebab-case")] pub enum RuleLevel { Off, Warning, Error }`
  - `#[derive(Deserialize, Default, Debug)] #[serde(deny_unknown_fields)] pub struct FileConfig { pub transaction: Option<TransactionSetting>, pub fail_on: Option<FailOnSetting>, #[serde(default)] pub exclude: Vec<String>, #[serde(default)] pub rules: BTreeMap<String, RuleLevel> }`. Las claves van en snake_case, como en spec §8 (`fail_on`). `TransactionSetting` acepta `"per-unit"` o `"none"` y `FailOnSetting` acepta `"error"` o `"warning"`; ambos enums llevan `rename_all = "kebab-case"`.
  - `#[derive(Debug, thiserror::Error)] pub enum ConfigError` con:
    - `Io { path, source }`.
    - `Invalid { path, message }`, con el mensaje de `toml`, que nombra la clave.
    - `UnknownRule { path, rule }`, con `{path}: unknown rule "{rule}" in [rules]`.
    - `BadGlob { pattern, message }`.
  - `pub fn find_config(start: &Path) -> Option<PathBuf>`: busca `lockline.toml` en `start` y en sus padres.
  - `pub fn load_config(path: &Path) -> Result<FileConfig, ConfigError>`: valida los IDs de `[rules]` contra `rules::infos()`.
  - `pub fn exclude_set(patterns: &[String]) -> Result<globset::GlobSet, ConfigError>`
  - `Config` (en `analyze.rs`) gana `pub rules: HashMap<String, RuleLevel>`. El analizador aplica `Off` (descarta el hallazgo) y `Warning`/`Error` (cambia la severidad) a todos los hallazgos, incluidos los de unidad.
- `main`:
  - Busca la configuración desde el directorio actual.
  - Los flags de clap pasan a `Option` para que los explícitos ganen sobre el archivo (precedencia: CLI > toml > defaults).
  - `exclude` se aplica sobre `MigrationUnit.name`; `<stdin>` nunca se excluye.
  - Cualquier `ConfigError` ⇒ `error: {e}` y exit 2.

- [ ] **Step 1: Tests que fallan**
  - `load_valid_config`, con el ejemplo de spec §8.
  - `unknown_rule_is_an_error` (Review Focus 4): `[rules] create-index-not-concurently = "off"` ⇒ `UnknownRule`, y el mensaje contiene la ruta y el ID.
  - `unknown_key_is_an_error`: `fail_onn = "error"` ⇒ `Invalid`, y el mensaje contiene `fail_onn`.
  - `find_config_walks_up`.
  - CLI (con `current_dir` en un tempdir):
    - `config_turns_rule_off`.
    - `config_raises_warning_to_error` ⇒ exit 1.
    - `cli_flag_overrides_config`: `--fail-on error` sobre un toml con `fail_on = "warning"`.
    - `exclude_skips_files`.
    - `bad_config_exits_two`.
- [ ] **Step 2:** `cargo test --test config --test cli` ⇒ FAIL · **Step 3:** implementar · **Step 4:** `cargo test` ⇒ PASS · **Step 5:** commit
```bash
git add Cargo.toml Cargo.lock src/config.rs src/analyze.rs src/main.rs src/lib.rs tests
git commit -m "feat(config): leer lockline.toml con niveles por regla, exclusiones y precedencia de flags"
```

---

### Task 15: Salida JSON, `--format` y README

**Files:**
- Create: `src/report/json.rs`
- Modify: `Cargo.toml` (`serde_json = "1"`), `src/report/mod.rs`, `src/main.rs`, `README.md`, `README.es.md`
- Test: `tests/json.rs` (+ snapshot), `tests/cli.rs`

**Interfaces:**
- Produces: `pub fn render_json(report: &Report) -> String` (JSON con sangría y `\n` final), con structs `Serialize` propias de `json.rs`, desacopladas del modelo interno. El esquema es estable:
```json
{
  "version": 1,
  "files": [{
    "path": "m.sql",
    "findings": [{"rule": "parse-error", "severity": "error", "message": "…", "help": null, "line": 3}],
    "statements": [{
      "line": 1, "col": 1, "end_line": 1, "text": "CREATE INDEX …",
      "analyzed": true,
      "effects": [{"table": "users", "lock": "SHARE", "blocks_reads": false, "blocks_writes": true,
                   "cost": "scan", "new_table": false, "note": "index build"}],
      "findings": [{"rule": "…", "severity": "error", "message": "…", "help": "…", "line": null}],
      "suppressed": [{"rule": "…", "reason": "…"}]
    }],
    "transactions": [{"ends_after_line": null, "statement_count": 2, "locks": [{"table": "users", "lock": "ACCESS EXCLUSIVE"}]}]
  }],
  "summary": {"errors": 1, "warnings": 0, "suppressed": 0, "files": 1, "statements": 1, "not_analyzed": 0}
}
```
  - `analyzed` es `false` solo para `NotAnalyzed`; `NoTableLock` da `true` con `effects: []`.
  - `cost` va en minúsculas (`metadata`, `scan`, `rewrite`, `unknown`).
  - `ends_after_line` es la línea del `COMMIT`, o `null` para el fin implícito.
- CLI: `--format <human|json>`, con `human` por defecto. Con `json` se ignoran `--quiet` y `--color`. El código de salida no cambia.
- README:
  - Tabla con las 20 reglas.
  - Secciones "Configuration" (`lockline.toml`), "Suppressing a finding", "JSON output" y "Rule docs" (`lockline explain`).
  - El estado pasa a "milestone 2 of 4".
  - Regenerar la salida del ejemplo.
  - Agregar a `README.es.md` una línea por cada sección nueva.

- [ ] **Step 1: Tests que fallan**
  - `json_snapshot` sobre el mismo reporte del snapshot humano del hito 1.
  - `json_is_valid_and_stable`: parsearlo con `serde_json::Value` y comprobar `version == 1`, `summary.errors` y `files[0].statements[0].effects[0].lock == "SHARE"`.
  - CLI `format_json`: exit 1 y stdout parseable.
- [ ] **Step 2:** `cargo test --test json --test cli` ⇒ FAIL · **Step 3:** implementar · **Step 4:** `cargo insta test --review`, `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` ⇒ todo en verde · **Step 5:** commit
```bash
git add Cargo.toml Cargo.lock src/report src/main.rs tests README.md README.es.md
git commit -m "feat(report): agregar salida json estable y actualizar el readme del hito 2"
```

---

## Lo que queda para los hitos siguientes

- **Hito 3:** `tests/pg_verify.rs` contra Postgres 14–17. Debe verificar cada fila de las tablas de las Tasks 5 y 6, en particular `ATTACH PARTITION` (AccessExclusive sobre la partición), `COMMENT ON` y `SET (storage params)`, que hoy se basan en la documentación.
- **Hito 4:** adaptador Laravel, SARIF, `github`, `--history` (reutiliza `Schema::observe` sin reportar), GitHub Action y release.
