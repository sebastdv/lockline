# lockline — diseño

> Fecha: 2026-10-16 · Estado: borrador para aprobación
> El usuario pidió saltarse las preguntas; **todas las decisiones de este documento son mías**
> salvo que se indique lo contrario. Lo que pidió el usuario: un proyecto de portafolio público
> en GitHub, que no sea lo típico, que se valore hoy, y que sea el primero de una serie
> ("linter de migraciones para Postgres: Rust o Go, análisis de SQL, CLI, GitHub Action").

## 1. Qué es y por qué

`lockline` es una CLI que lee migraciones SQL de PostgreSQL **antes de que lleguen a
producción** y, para cada sentencia, dice:

- **qué lock toma** y sobre qué tabla (p. ej. `ACCESS EXCLUSIVE on users`),
- **qué bloquea** en lenguaje humano ("bloquea lecturas y escrituras"),
- si **reescribe** la tabla o la **recorre entera** (el costo que crece con el tamaño),
- y si es un patrón peligroso, **por qué** y **cuál es la alternativa segura** (SQL sugerido).

El problema real: una migración que en local tarda 20 ms, en producción con una tabla de
50 M de filas deja la app sin poder escribir durante minutos (o, peor, un `ALTER TABLE`
esperando un lock pone en cola *todas* las consultas detrás de él). Es un error común,
caro y fácil de prevenir en CI.

### Por qué es buen proyecto de portafolio
- Muestra **Rust**, análisis de un AST real (el parser oficial de Postgres vía `pg_query`),
  conocimiento profundo de Postgres, diseño de CLI, testing serio y CI/CD.
- Es útil de verdad: se puede usar en los propios proyectos (Laravel, TypeORM, Prisma).

### Antecedentes (prior art) y en qué se diferencia
Existen herramientas similares y **el README debe reconocerlas** con honestidad:
`squawk` (Rust, linter con muchas reglas), `eugene` (Rust, traza locks ejecutando contra una
base), `strong_migrations` (Ruby/Rails). `lockline` no compite en número de reglas; su ángulo es:

1. **Explica el lock de cada sentencia**, no solo las que violan una regla. La salida sirve
   para aprender, no solo para bloquear el PR.
2. **El catálogo de locks está verificado contra Postgres real en CI** (matriz 14–17): un test
   ejecuta cada sentencia del catálogo en una transacción y consulta `pg_locks`. Cada
   afirmación de la herramienta está respaldada por un test.
3. **Entiende el contexto de la migración**: tablas creadas en la misma migración (no tienen
   tráfico → no se marcan), transacciones (un `CONCURRENTLY` o un `VACUUM` dentro de transacción falla;
   los locks se acumulan hasta el `COMMIT`) y si hay `lock_timeout` antes de un lock fuerte.
4. **Adaptador para Laravel** (`php artisan migrate --pretend | lockline check --from laravel -`),
   el stack del autor, que ninguna de las anteriores cubre.
5. **Salidas para CI**: humana, JSON, SARIF (code scanning de GitHub) y anotaciones de
   GitHub Actions, más una GitHub Action lista para usar.

## 2. Objetivos, no-objetivos y anti-objetivos

**Objetivos (v1)**
- CLI `lockline` que analiza archivos `.sql`, directorios y stdin.
- Catálogo de locks para las sentencias DDL/DML comunes en migraciones (§5).
- ~15 reglas (§6) con mensaje, explicación y sugerencia segura.
- Conciencia de contexto: tablas nuevas, transacciones, `lock_timeout`.
- Supresión por comentario y configuración por archivo `lockline.toml`.
- Salidas: `human`, `json`, `sarif`, `github`.
- Adaptador de entrada `laravel` (salida de `migrate --pretend`).
- Verificación del catálogo contra Postgres 14–17 en CI.
- GitHub Action, binarios de release (macOS arm64/x86_64, Linux x86_64/arm64), README con demo.

**No-objetivos (v1)**
- Conectarse a la base de producción o estimar tiempos por tamaño de tabla.
- Parsear migraciones en TypeScript/PHP/Python directamente (solo SQL, o la salida SQL que
  genera la herramienta de migraciones: `--pretend`, `sqlmigrate`, archivos de Prisma).
- Otros motores (MySQL, SQLite).
- Playground web/WASM (posible extra futuro).
- Reescribir automáticamente los archivos (`--fix`): la v1 *sugiere* SQL, no lo aplica.
- Postgres < 14 (fuera de soporte).

**Anti-objetivos (lo que lo haría un fracaso aunque funcione)**
- **Falsos positivos** que hagan que la gente lo desactive. Ante la duda, una regla es
  `warning`, no `error`, y siempre se puede suprimir con motivo.
- **Afirmar algo sobre locks que no esté verificado.** Si una sentencia no está en el
  catálogo, se dice "no analizada", nunca se inventa.
- Ser un clon de `squawk`. Si una decisión solo lo acerca a ser "squawk con otro nombre",
  se descarta.
- Un repo que no se pueda probar en 1 minuto: el README tiene instalación de una línea y un
  ejemplo que se copia y pega.

## 3. Restricciones

- **Lenguaje:** Rust estable (edición 2024). Parser: crate `pg_query` 6.x (gramática de
  Postgres 17; sin `protoc` instalado usa los bindings pregenerados).
- **Entorno del autor:** macOS, Docker Desktop (VM de 4 GB: un solo contenedor de Postgres a
  la vez en local). Rust **no está instalado**: el plan empieza por instalar `rustup`.
- **Licencia:** MIT OR Apache-2.0 (convención de Rust).
- **Idioma:** código, mensajes de la CLI, README y docs de reglas en **inglés** (repo público);
  `README.es.md` breve en español. Diseño y plan en español.
- **Commits:** Conventional Commits en español, una sola línea (convención del autor).
- **Repo público** en GitHub: lo crea el autor (no hay `gh` instalado). Publicar en crates.io
  es decisión del autor, no parte del plan automático.
- **Nada de código, datos ni nombres de Sódeker** en el repo: los fixtures de Laravel se
  generan con una app Laravel vacía o se escriben a mano.

## 4. Experiencia de uso

```bash
lockline check migrations/                       # todos los .sql, en orden por nombre
lockline check db/2026_10_16_add_index.sql       # un archivo
cat migration.sql | lockline check -             # stdin
php artisan migrate --pretend | lockline check --from laravel -
lockline check migrations/ --format sarif > lockline.sarif
lockline explain create-index-not-concurrently   # documentación de una regla
lockline rules                                   # lista de reglas con severidad
```

Salida humana (ilustrativa):

```
migrations/0007_add_email_index.sql
  3:1  CREATE INDEX idx_users_email ON users (email)
       lock   SHARE on users — blocks writes, allows reads
       cost   full table scan (index build)
  error[create-index-not-concurrently] Building an index without CONCURRENTLY blocks
       all writes to "users" until the build finishes.
       help: CREATE INDEX CONCURRENTLY idx_users_email ON users (email);
             (CONCURRENTLY cannot run inside a transaction block)

  5:1  ALTER TABLE users ADD COLUMN nickname text
       lock   ACCESS EXCLUSIVE on users — blocks reads and writes
       cost   metadata only (fast)

1 error, 0 warnings in 1 file (2 statements analyzed)
```

- `--verbose` / por defecto: la salida humana muestra **todas** las sentencias con su lock
  (es el punto 1 del diferenciador). `--quiet` muestra solo hallazgos.
- **Códigos de salida:** `0` sin hallazgos que fallen; `1` hay hallazgos ≥ umbral
  (`--fail-on error|warning`, por defecto `error`); `2` error de uso, IO o configuración.
- Un archivo que no parsea produce un hallazgo `parse-error` (severidad error) con el
  mensaje del parser y la línea si el parser la entrega (`pg_query` falla el archivo entero
  ante un error de sintaxis; no hay análisis parcial), y el análisis sigue con los demás
  archivos.

## 5. Modelo: locks, costo y catálogo

**Modos de lock** (los 8 de Postgres, de menor a mayor): `ACCESS SHARE`, `ROW SHARE`,
`ROW EXCLUSIVE`, `SHARE UPDATE EXCLUSIVE`, `SHARE`, `SHARE ROW EXCLUSIVE`, `EXCLUSIVE`,
`ACCESS EXCLUSIVE`. La matriz de conflictos se codifica tal cual la documentación oficial
("Explicit Locking", tabla de conflictos). De ella se derivan:
- `blocks_reads` = conflicta con `ACCESS SHARE` (lo que toma un `SELECT`),
- `blocks_writes` = conflicta con `ROW EXCLUSIVE` (lo que toman `INSERT/UPDATE/DELETE`).

**Costo** de una sentencia sobre una tabla: `Metadata` (rápido), `Scan` (recorre la tabla sin
reescribir, p. ej. validar un `CHECK`), `Rewrite` (reescribe la tabla y sus índices),
`Unknown`.

**Efecto** = (tabla, modo de lock, costo, notas). Una sentencia produce cero o más efectos
(un `ADD FOREIGN KEY` toma lock en la tabla y en la referenciada). Para `ALTER TABLE` con
varios subcomandos se toma el lock máximo y el costo máximo.

**Catálogo v1** (cada fila debe tener test de verificación contra Postgres real, §8):

| Sentencia | Lock | Costo |
|---|---|---|
| `CREATE INDEX` | SHARE | Scan |
| `CREATE INDEX CONCURRENTLY` | SHARE UPDATE EXCLUSIVE | Scan (no bloquea escrituras) |
| `DROP INDEX` | ACCESS EXCLUSIVE (tabla) | Metadata |
| `DROP INDEX CONCURRENTLY` | SHARE UPDATE EXCLUSIVE | Metadata |
| `REINDEX` (tabla/índice) | SHARE (tabla) | Rewrite del índice |
| `REINDEX CONCURRENTLY` | SHARE UPDATE EXCLUSIVE | Rewrite del índice |
| `ALTER TABLE ... ADD COLUMN` (default no volátil o sin default) | ACCESS EXCLUSIVE | Metadata |
| `ALTER TABLE ... ADD COLUMN` (default volátil, `serial`, identity, generated stored) | ACCESS EXCLUSIVE | Rewrite |
| `ALTER TABLE ... ADD COLUMN` con restricción en línea (`REFERENCES`, `UNIQUE`, `PRIMARY KEY`, `CHECK`, `EXCLUDE`) | ACCESS EXCLUSIVE (+ SHARE ROW EXCLUSIVE en la tabla referenciada) | Scan |
| `ALTER TABLE ... DROP COLUMN` | ACCESS EXCLUSIVE | Metadata |
| `ALTER TABLE ... ALTER COLUMN TYPE` | ACCESS EXCLUSIVE | Rewrite (salvo ampliaciones seguras conocidas) |
| `ALTER TABLE ... ALTER COLUMN SET NOT NULL` | ACCESS EXCLUSIVE | Scan |
| `ALTER TABLE ... ALTER COLUMN DROP NOT NULL / SET DEFAULT / DROP DEFAULT` | ACCESS EXCLUSIVE | Metadata |
| `ALTER TABLE ... ADD CONSTRAINT CHECK` | ACCESS EXCLUSIVE | Scan (Metadata si `NOT VALID`) |
| `ALTER TABLE ... ADD CONSTRAINT FOREIGN KEY` | SHARE ROW EXCLUSIVE (ambas tablas) | Scan (Metadata si `NOT VALID`) |
| `ALTER TABLE ... ADD CONSTRAINT UNIQUE / PRIMARY KEY` | ACCESS EXCLUSIVE | Scan (construye índice) — Metadata con `USING INDEX` |
| `ALTER TABLE ... VALIDATE CONSTRAINT` | SHARE UPDATE EXCLUSIVE | Scan |
| `ALTER TABLE ... RENAME (tabla/columna/constraint)` | ACCESS EXCLUSIVE | Metadata |
| `ALTER TABLE ... SET (storage params)` / `SET STATISTICS` | SHARE UPDATE EXCLUSIVE | Metadata |
| `ALTER TABLE ... ATTACH PARTITION` | SHARE UPDATE EXCLUSIVE (padre) | Scan (de la partición) |
| `ALTER TABLE ... DETACH PARTITION` | ACCESS EXCLUSIVE | Metadata |
| `ALTER TABLE ... DETACH PARTITION CONCURRENTLY` | SHARE UPDATE EXCLUSIVE | Metadata |
| `DROP TABLE` | ACCESS EXCLUSIVE | Metadata |
| `TRUNCATE` | ACCESS EXCLUSIVE | Metadata |
| `VACUUM FULL`, `CLUSTER` | ACCESS EXCLUSIVE | Rewrite |
| `VACUUM`, `ANALYZE` | SHARE UPDATE EXCLUSIVE | Scan |
| `REFRESH MATERIALIZED VIEW` | ACCESS EXCLUSIVE | Rewrite |
| `REFRESH MATERIALIZED VIEW CONCURRENTLY` | EXCLUSIVE | Rewrite |
| `CREATE TRIGGER` | SHARE ROW EXCLUSIVE | Metadata |
| `INSERT / UPDATE / DELETE` | ROW EXCLUSIVE | Unknown (depende del `WHERE`) |
| `CREATE TABLE` con `REFERENCES` a una tabla existente | SHARE ROW EXCLUSIVE (tabla referenciada) | Metadata |
| `CREATE TABLE ... PARTITION OF` | ACCESS EXCLUSIVE (tabla padre) | Metadata |
| `CREATE TABLE ... INHERITS` | SHARE UPDATE EXCLUSIVE (tabla padre) | Metadata |
| `CREATE OR REPLACE VIEW` sobre una vista existente | ACCESS EXCLUSIVE (la vista) | Metadata |
| `COMMENT ON TABLE/COLUMN` | SHARE UPDATE EXCLUSIVE | Metadata |
| `CREATE TABLE` (sin lo anterior), `CREATE TYPE`, `CREATE FUNCTION`, `CREATE VIEW`, `SET`, `BEGIN/COMMIT` | sin lock relevante sobre tablas existentes | — |

Cualquier sentencia fuera del catálogo se informa como `not analyzed` (sin inventar lock).
Las celdas del catálogo son la hipótesis inicial; **si el test contra Postgres real dice otra
cosa, gana el test** y se corrige el catálogo.

**Volatilidad del default:** lista explícita de funciones volátiles conocidas
(`random`, `gen_random_uuid`, `uuid_generate_v1/v4`, `clock_timestamp`, `timeofday`,
`nextval`, `txid_current`, …) ⇒ Rewrite. Funciones estables/inmutables conocidas (`now`,
`current_timestamp`, `current_date`, literales, casts de literales) ⇒ Metadata. Función
desconocida ⇒ Metadata + hallazgo `warning` "no se puede saber si es volátil".

**Ampliaciones de tipo seguras** (no reescriben): `varchar(n)`→`varchar(m≥n)`,
`varchar(n)`→`varchar`/`text`, `numeric(p,s)`→`numeric(p2≥p,s)`, `varbit(n)`→`varbit(m≥n)`.
Solo se pueden reconocer si el tipo anterior es conocido (ver §7, esquema virtual); si no, la
regla reporta con un mensaje que menciona la excepción.

## 6. Reglas v1

Identificadores en kebab-case (estables, se usan en supresiones y config).

| Regla | Severidad | Dispara cuando |
|---|---|---|
| `create-index-not-concurrently` | error | `CREATE INDEX` sin `CONCURRENTLY` sobre tabla existente |
| `drop-index-not-concurrently` | warning | `DROP INDEX` sin `CONCURRENTLY` |
| `reindex-not-concurrently` | error | `REINDEX` sin `CONCURRENTLY` |
| `cannot-run-in-transaction` | error | `CREATE/DROP INDEX CONCURRENTLY`, `REINDEX CONCURRENTLY`, `DETACH PARTITION CONCURRENTLY` o `VACUUM` dentro de una transacción (explícita o implícita, §7). El *help* menciona cómo desactivar la transacción en Laravel (`public $withinTransaction = false;`) |
| `add-column-rewrite` | error | `ADD COLUMN` cuyo costo es Rewrite |
| `add-column-not-null-without-default` | error | `ADD COLUMN ... NOT NULL` sin `DEFAULT` sobre tabla existente (falla si la tabla tiene filas) |
| `add-column-unknown-default` | warning | `ADD COLUMN` con default de volatilidad desconocida |
| `alter-column-type` | error | `ALTER COLUMN TYPE` que no es una ampliación segura conocida |
| `set-not-null` | error | `SET NOT NULL` sin un `CHECK (col IS NOT NULL)` validado antes en la misma corrida |
| `add-check-without-not-valid` | error | `ADD CONSTRAINT CHECK` sin `NOT VALID` |
| `add-foreign-key-without-not-valid` | error | `ADD FOREIGN KEY` sin `NOT VALID` |
| `add-unique-without-index` | error | `ADD UNIQUE/PRIMARY KEY` sin `USING INDEX` |
| `rewrite-table` | error | `VACUUM FULL`, `CLUSTER` |
| `refresh-matview-not-concurrently` | warning | `REFRESH MATERIALIZED VIEW` sin `CONCURRENTLY` |
| `missing-lock-timeout` | warning | se toma `ACCESS EXCLUSIVE` sobre tabla existente sin `SET lock_timeout` previo en la misma sesión/transacción |
| `rename-object` | warning | `RENAME` de tabla o columna (rompe el código desplegado que aún usa el nombre viejo) |
| `drop-object` | warning | `DROP TABLE` / `DROP COLUMN` (misma razón; sugiere expand/contract) |
| `backfill-in-migration` | warning | `UPDATE`/`DELETE` sin `WHERE` sobre tabla existente |
| `parse-error` | error | el archivo no parsea |

**Regla general:** ninguna regla dispara sobre una tabla **creada en la misma unidad de
migración** (no tiene tráfico ni filas). Excepción: `cannot-run-in-transaction`, que es un
error de ejecución sin importar la tabla. Las sentencias sobre tablas nuevas igual muestran
su lock en la salida humana, con la nota `(new table)`.

Cada regla tiene: `id`, severidad por defecto, mensaje corto, explicación larga (para
`lockline explain` y `docs/rules/<id>.md`) y, cuando aplica, **SQL sugerido** construido a
partir de la sentencia real (p. ej. el mismo `CREATE INDEX` con `CONCURRENTLY`; el
`ADD CONSTRAINT ... NOT VALID` seguido de `VALIDATE CONSTRAINT`).

## 7. Contexto de análisis

**Unidad de migración:** un archivo `.sql`, la entrada de stdin, o cada migración dentro de
la salida de Laravel. El estado de "tablas nuevas" y transacción se reinicia por unidad; el
esquema virtual se acumula en toda la corrida (en orden).

**Transacciones.** Modo configurable `transaction = "per-unit" | "none"`:
- `per-unit` (por defecto: así corren Laravel —en Postgres—, Flyway, Rails, Django y la
  mayoría): la unidad entera está en una transacción implícita.
- `none`: autocommit (como `psql -f` sin `--single-transaction`).
- Un `BEGIN`/`COMMIT`/`ROLLBACK` explícito en el SQL siempre manda sobre el modo.
- Dentro de una transacción los locks **se acumulan hasta el COMMIT**: la salida humana
  muestra, al final de cada transacción, la lista de locks retenidos
  (`holds until commit: ACCESS EXCLUSIVE on users, SHARE on orders`).

**`lock_timeout`:** se registra `SET lock_timeout = ...` y `SET LOCAL lock_timeout`
(este último solo hasta el fin de la transacción). Un valor `0` cuenta como "sin timeout".

**Esquema virtual (mínimo):** a partir de `CREATE TABLE` y `ADD/DROP/ALTER COLUMN` vistos en
la corrida se lleva `tabla → columnas → tipo`, más los `CHECK (col IS NOT NULL)` validados.
Se usa para: ampliaciones de tipo seguras y `set-not-null`. Opción `--history <dir>`: lee
los `.sql` de ese directorio **anteriores** (por nombre) a los analizados solo para
construir el esquema, sin reportar nada. Así en CI se analizan solo los archivos nuevos del
PR con el contexto de los anteriores.

**Nombres de tabla:** se normalizan como Postgres (sin comillas → minúsculas); el esquema
(`public.users` vs `users`) se compara ignorando `public` cuando no viene.

## 8. Arquitectura

Crate único con `lib` + `bin` (la librería permite testear todo sin el binario):

```
src/
  main.rs            CLI (clap): check, explain, rules
  lib.rs             API pública: analyze(units, config) -> Report
  input/             fuentes → Vec<MigrationUnit { name, path, sql }>
    files.rs           archivos, directorios (orden por nombre), stdin
    laravel.rs         parser de la salida de `migrate --pretend`
  parse.rs           pg_query → Vec<Statement { node, text, line, col, byte_range }>
  locks.rs           LockMode + matriz de conflictos + blocks_reads/blocks_writes
  catalog.rs         Statement → Vec<Effect { table, lock, cost, note }>
  context.rs         estado: tablas nuevas, transacción, lock_timeout, esquema virtual
  rules/             trait Rule + un archivo por regla
  suppress.rs        comentarios `-- lockline:ignore <rule-id> <motivo>`
  config.rs          lockline.toml
  report/            human.rs, json.rs, sarif.rs, github.rs
tests/
  fixtures/          *.sql por regla (casos que disparan y que no)
  snapshots/         insta
  cli.rs             assert_cmd sobre el binario
  pg_verify.rs       verificación del catálogo contra Postgres real (ignorado sin DB)
docs/rules/          una página por regla (generada o escrita, coincide con `explain`)
action.yml           GitHub Action compuesta
```

**Supresión:** `-- lockline:ignore <rule-id>[,<rule-id>] <motivo>` en la línea anterior a la
sentencia la suprime; `-- lockline:ignore-file <rule-id> <motivo>` en cualquier parte del
archivo la suprime para todo el archivo. El motivo es obligatorio (sin motivo → `warning`
`ignore-without-reason`). Las suprimidas cuentan en el resumen (`2 suppressed`).

**Configuración `lockline.toml`** (busca en el directorio actual y padres):
```toml
transaction = "per-unit"     # o "none"
fail_on = "error"            # o "warning"
exclude = ["migrations/legacy/**"]
[rules]
drop-object = "off"          # off | warning | error
missing-lock-timeout = "error"
```
Precedencia: flags de CLI > `lockline.toml` > defaults.

**Formatos:**
- `human`: con color si es TTY (`NO_COLOR` respetado).
- `json`: estable y documentado (`{ version, files: [{ path, statements: [{ line, col, text, effects, findings }] }], summary }`).
- `sarif`: SARIF 2.1.0, una `rule` por regla con `helpUri` a `docs/rules/<id>.md`.
- `github`: `::error file=...,line=...,title=...::mensaje` para anotar el PR.

**Verificación contra Postgres real (`tests/pg_verify.rs`):** por cada fila del catálogo,
un caso con setup (crear tablas), la sentencia, y el lock esperado. El test abre una
transacción, ejecuta la sentencia, consulta `pg_locks` unido a `pg_class` y compara el modo
más fuerte tomado sobre la tabla con el del catálogo (las sentencias `CONCURRENTLY`, que no
corren en transacción, se verifican desde una segunda conexión mientras la primera está
en curso, o se marcan como verificadas por documentación si no hay forma estable de
observarlas — **decisión del implementador**, documentada en el test). Se activa con
`LOCKLINE_TEST_DATABASE_URL`; en CI corre en matriz Postgres 14, 15, 16 y 17 con servicios
de GitHub Actions; en local con `docker compose up` (un solo contenedor).

## 9. Entregables y CI

- **CI (GitHub Actions):** `fmt --check`, `clippy -D warnings`, tests, y la matriz de
  verificación contra Postgres.
- **Release:** al crear un tag `v*`, workflow que compila binarios para macOS
  (arm64, x86_64) y Linux (x86_64, arm64 musl), los sube al release con checksums.
  Herramienta: `cargo-dist` (**mi decisión**; si se complica, un workflow con matriz propia).
- **GitHub Action** (`action.yml`, compuesta): descarga el binario del release, ejecuta
  `lockline check` con `--format github` sobre la ruta dada y opcionalmente sube SARIF.
- **Dogfooding:** el propio repo tiene un workflow de ejemplo que corre la Action sobre
  `examples/migrations/`.
- **README (inglés):** qué problema resuelve en 3 líneas, GIF de demo (grabado con `vhs`),
  instalación (`cargo install lockline`, binario, Action), ejemplo, tabla de reglas,
  sección "How is this different from squawk/eugene/strong_migrations", cómo se verifica el
  catálogo. `README.es.md` corto.

## 10. Hitos

1. **Núcleo:** toolchain, parser, modelo de locks, catálogo para índices y `ADD COLUMN`,
   contexto (tablas nuevas + transacciones), primeras reglas (`create-index-not-concurrently`,
   `cannot-run-in-transaction`, `add-column-rewrite`, `parse-error`), salida humana,
   códigos de salida, tests con fixtures y snapshots.
2. **Cobertura:** resto del catálogo y de las reglas, `lock_timeout`, esquema virtual,
   supresiones, `lockline.toml`, `explain`/`rules`, JSON, docs de reglas.
3. **Verificación:** `pg_verify.rs`, `docker-compose.yml`, CI con matriz de Postgres.
4. **Ecosistema:** adaptador Laravel, SARIF y `github`, `--history`, GitHub Action, release
   con binarios, README con demo.

Cada hito deja el repo en un estado publicable (tests verdes, README al día).

## 11. Decisiones que dejo al implementador

- Estructura interna exacta de tipos y nombres de funciones (respetando los módulos de §8).
- Biblioteca de colores para la salida humana (`anstream`/`owo-colors` o similar) y de
  diagnósticos (no es obligatorio `miette`).
- Cómo verificar los locks de las sentencias `CONCURRENTLY` (§8).
- Lista exacta de funciones volátiles/estables más allá de las nombradas (documentarla en
  el código y en `docs/rules/add-column-rewrite.md`).
- Texto exacto de mensajes y explicaciones (en inglés, claros, sin jerga innecesaria).

## 12. Decisiones mías que el autor debería revisar

- Nombre `lockline` (libre en crates.io a 2026-10-16).
- Rust + `pg_query` (frente a Go).
- Postgres 14–17 como rango soportado; 18 cuando `pg_query` incluya su gramática.
- `transaction = "per-unit"` por defecto.
- Repo en inglés con README en español aparte.
- Mostrar el lock de todas las sentencias por defecto (`--quiet` para solo hallazgos).
- Licencia MIT OR Apache-2.0.
