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

Estado: en desarrollo (hito 1 de 4). Licencia MIT o Apache-2.0.
