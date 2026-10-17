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
- `--format json` entrega un reporte estable para otras herramientas.

- Cada lock y cada reescritura del catálogo se verifica contra PostgreSQL 14, 15, 16 y 17
  reales en CI (`tests/pg_verify.rs`).

Estado: en desarrollo (hito 3 de 4). Licencia MIT o Apache-2.0.
