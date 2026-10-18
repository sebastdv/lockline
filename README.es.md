# lockline (español)

> La documentación principal está en inglés: [README.md](README.md).

**Sabe qué bloquea tu migración de PostgreSQL antes de que llegue a producción.**

`lockline` lee las migraciones SQL con el parser real de PostgreSQL y, para cada sentencia,
dice qué lock toma, qué bloquea (lecturas, escrituras) y si reescribe o recorre la tabla.
Si el patrón es peligroso, explica por qué y sugiere la versión segura.

```bash
lockline check migrations/
```

- Las sentencias sobre tablas creadas en la misma migración no se marcan: aún no tienen
  filas ni tráfico.
- Entiende transacciones: `CREATE INDEX CONCURRENTLY` o `VACUUM` dentro de una transacción
  fallan, y lo avisa.
- No adivina: lo que no está en su catálogo se informa como `not analyzed`.
- Código de salida `1` si hay errores, para bloquear el PR en CI.

- Avisa una vez por archivo si se toma un lock `ACCESS EXCLUSIVE` sin `SET lock_timeout`, y
  muestra los locks que una transacción retiene hasta el `COMMIT`.
- `lockline rules` lista las 20 reglas y `lockline explain <regla>` explica cada una.
- Un hallazgo se suprime con `-- lockline:ignore <regla> <motivo>` en la línea anterior.
- `lockline.toml` fija la configuración del proyecto (transacciones, umbral, exclusiones y
  nivel de cada regla).
- `--format json` entrega un reporte estable para otras herramientas; `--format sarif`, el
  formato de code scanning de GitHub, y `--format github`, anotaciones en el PR.
- Lee la salida de Laravel: `php artisan migrate --pretend | lockline check --from laravel -`.
  Cada migración se reporta con el nombre de su archivo PHP.
- `--history` aprende el esquema de migraciones anteriores (una carpeta de `.sql` o un
  volcado como el de `php artisan schema:dump`) sin reportarlas.
- Trae una GitHub Action (`uses: sebastdv/lockline@v0.1.0`) que instala un binario verificado
  con su checksum y anota el PR. Hay binarios para macOS y Linux en cada release.

- Los locks, las reescrituras y las afirmaciones de "solo metadata" de las sentencias de
  `tests/pg_verify.rs` se verifican contra PostgreSQL 14, 15, 16 y 17 reales (ver los límites
  conocidos en el README en inglés).

Estado: v0.1 completa (los cuatro hitos). Licencia MIT o Apache-2.0.
