# Convenciones del proyecto

## Binarios y crates

| Binario | Crate | Función |
|---|---|---|
| `onepackd` | `onepack-server` | Servidor del registro. |
| `onepack` | `onepack-cli` | CLI administrativo. |

Librerías: `onepack-core`, `onepack-nuget`, `onepack-storage`, `onepack-api-client`. Las dependencias permitidas entre ellas están en [ADR-003](../roadmap/adr-mvp.md#adr-003) y las verifica `scripts/check-crate-deps.sh` en CI.

## Toolchain

- Versión fijada en `rust-toolchain.toml`. Ese valor es también el MSRV (`rust-version` del workspace).
- Para subir la versión se cambian ambos valores en el mismo PR.
- `unsafe` está prohibido en todo el workspace (`unsafe_code = "forbid"`).

## Errores

Todo error que llegue a un usuario (API `/api/v1`, CLI o log de operación) tiene:

| Campo | Ejemplo | Regla |
|---|---|---|
| `code` | `AUTH_SCOPE_MISSING` | `SCREAMING_SNAKE_CASE`, prefijo por área, **estable**: no se renombra una vez publicado. |
| `message` | `La credencial es válida, pero no permite publicar en "internal".` | Legible y sin secretos. |
| `action` | `Requiere el permiso packages:publish.` | Opcional. Indica qué hacer. |
| `request_id` | `req_01J…` | Presente en toda respuesta HTTP. |

Formato JSON en `/api/v1`:

```json
{
  "error": {
    "code": "AUTH_SCOPE_MISSING",
    "message": "La credencial es válida, pero no permite publicar en \"internal\".",
    "action": "Requiere el permiso packages:publish.",
    "request_id": "req_01J..."
  }
}
```

Los endpoints del protocolo NuGet responden con el código HTTP que exige el protocolo. Incluyen además `X-Request-Id`.

### Prefijos de código

| Prefijo | Área |
|---|---|
| `AUTH_` | Autenticación y autorización. |
| `FEED_` | Feeds. |
| `PKG_` | Paquetes y versiones (validación, duplicados, estados). |
| `LIMIT_` | Cuotas, límites de tamaño y de tasa. |
| `STORAGE_` | Base de datos y blobs. |
| `MAINT_` | Modo mantenimiento, backup, migraciones. |
| `CLI_` | Errores locales del CLI (configuración, keychain, entrada). |

Los secretos nunca aparecen en mensajes, logs ni salidas de diagnóstico.

## Códigos de salida del CLI

| Código | Significado |
|---|---|
| `0` | Éxito. |
| `1` | Error genérico o inesperado. |
| `2` | Uso incorrecto (argumentos o flags inválidos). |
| `3` | Autenticación fallida o credencial ausente. |
| `4` | Permiso denegado. |
| `5` | Recurso no encontrado. |
| `6` | Conflicto (p. ej. versión ya existente con contenido distinto). |
| `7` | Servidor no disponible, timeout o modo mantenimiento. |

La tabla es definitiva cuando se cierra la Fase 6. Después, cambiarla exige una versión mayor del CLI ([ADR-015](../roadmap/adr-mvp.md#adr-015)).

## Commits y ramas

- Commits con [Conventional Commits](https://www.conventionalcommits.org/): `feat(nuget): …`, `fix(storage): …`, `docs(adr): …`, `ci: …`, `chore: …`.
- Scope = nombre corto del crate (`core`, `nuget`, `storage`, `api-client`, `server`, `cli`) o área (`adr`, `roadmap`, `ci`).
- Ramas: `fase-N/<descripcion-corta>` para trabajo del roadmap y `fix/<descripcion>` para correcciones.
- Todo PR pasa por CI en verde antes de fusionarse en `main`.

## Decisiones

- Las decisiones de arquitectura van en [roadmap/adr-mvp.md](../roadmap/adr-mvp.md). Su ciclo de vida está descrito en ese documento.
- Un PR que contradiga un ADR `Aceptado` debe incluir el ADR que lo reemplaza.
