# Git Commit Instructions (EN title + ES body)

Estas instrucciones estandarizan commits y pull requests para cumplir normas comunes de GitHub y mantener claridad en el historial.

## Regla De Idioma

- Titulo del commit: en ingles.
- Cuerpo del commit: en espanol.
- Sugerencia de nombre de rama: en ingles.

## Commit Title Format (Conventional Commits)

Usa este patron:

`<type>(<scope>): <short english description>`

Reglas del titulo:

- Maximo recomendado: 72 caracteres.
- Escribir en modo imperativo (`add`, `fix`, `update`, `remove`).
- Mantener `type`, `scope` y descripcion en ingles.
- No terminar con punto final.

`type` permitidos:

- `feat`: new feature.
- `fix`: bug fix.
- `refactor`: internal change without expected behavior changes.
- `perf`: performance improvement.
- `docs`: documentation changes.
- `test`: tests or testing adjustments.
- `build`: dependencies, build system or tooling.
- `ci`: CI/CD pipelines or configuration.
- `chore`: general maintenance not covered above.

`scope` sugeridos para este repo:

- `admin`
- `orders`
- `stock`
- `products`
- `api`
- `http`
- `bootstrap`
- `i18n`

Ejemplos validos de titulo:

- `feat(admin): add manual stock sync action`
- `fix(orders): prevent duplicate export on status transition`
- `refactor(api): centralize bsale request headers`
- `docs(readme): clarify setup for local woocommerce`

## Commit Body (En Espanol)

Agrega cuerpo cuando el cambio no sea trivial.

Formato recomendado:

- Que cambia.
- Por que cambia.
- Impacto o consideraciones de compatibilidad.

Ejemplo:

```text
fix(stock): avoid sending negative quantity to bsale

Se valida el stock antes de enviar la solicitud y se limita a cero.
Esto evita rechazos de la API cuando WooCommerce presenta
stock negativo transitorio durante actualizaciones concurrentes.
```

## Referenciar Issues

Cuando aplique, agrega referencia al final del commit o en el PR:

- `Closes #123`
- `Fixes #123`
- `Refs #123`

Usa `Closes`/`Fixes` solo si realmente cierra el issue al mergear.

## Commits Que Se Deben Evitar

- `update`
- `changes`
- `fix stuff`
- commits mezclando cambios no relacionados
- commits gigantes que dificultan rollback

## Buenas Practicas Antes De Commit

1. Ejecutar validacion de sintaxis de PHP en archivos modificados: `php -l <archivo.php>`
2. Verificar manualmente el flujo impactado en WordPress/WooCommerce.
3. Revisar diff y eliminar ruido (debug, logs temporales, cambios accidentales).
4. Hacer commits pequenos y enfocados (un cambio logico por commit).

## Estandar De Pull Request En GitHub

Cada PR debe incluir:

- Objetivo del cambio.
- Resumen de lo implementado.
- Impacto funcional (admin, checkout, sync, API Bsale, etc.).
- Pasos de prueba manual.
- Capturas de pantalla si cambia UI del admin.
- Issues relacionados (`Closes #...` o `Refs #...`).

Checklist sugerido para el cuerpo del PR:

```markdown
## Summary
- ...

## Testing
- [ ] php -l en archivos PHP modificados
- [ ] Prueba manual en Settings > Bsale (si aplica)
- [ ] Prueba manual en flujo de pedidos/stock (si aplica)

## Related
- Closes #...
```

## Nota Final En El Mensaje

Como se usa `squash merging`, agrega siempre al final del mensaje una linea con nombre sugerido de rama en ingles:

- `Branch name suggestion: <type>/<scope>-<short-english-description>`

Ejemplos:

- `Branch name suggestion: fix/orders-duplicate-export`
- `Branch name suggestion: feat/admin-manual-stock-sync`
