# Operación y observabilidad

> Fase 7. Instalación, backup y actualización: [`docs/runbooks/`](runbooks/).
> Última actualización: 2026-10-04

## Sondas

| Ruta | Credencial | Respuesta |
|---|---|---|
| `GET /healthz` | no | `200 {"status":"ok"}` mientras el proceso atiende. |
| `GET /readyz` | no | `200` si la base responde y el directorio de datos es accesible; `503` si no, con el detalle de cada comprobación. Durante un backup sigue en `200` e informa del mantenimiento. |

Están en el listener principal (para balanceadores y orquestadores) y en el de operación. No revelan datos del registro. Son las únicas rutas sin autenticación del listener principal.

## Métricas

`--ops-listen 127.0.0.1:9464` (`ONEPACK_OPS_LISTEN`) abre un segundo listener, sin autenticación, con `/metrics` en formato Prometheus y las sondas. Ábrelo solo a una red interna.

| Métrica | Tipo | Etiquetas |
|---|---|---|
| `onepack_http_requests_total` | contador | `surface` (`nuget`, `admin`, `probe`), `method`, `status` |
| `onepack_http_request_duration_seconds` | histograma | `surface` |
| `onepack_errors_total` | contador | `code` (códigos estables: `RATE_LIMITED`, `PACKAGE_UNSAFE_PATH`, `FEED_QUOTA_STORAGE`, `MAINTENANCE`…) |
| `onepack_uploads_total` | contador | `outcome` (`published`, `conflict`, `rejected`) |
| `onepack_uploads_in_flight` | gauge | |
| `onepack_maintenance` | gauge | 1 durante un mantenimiento |
| `onepack_data_dir_free_bytes` | gauge | |
| `onepack_build_info` | gauge | `version` |

Las etiquetas tienen cardinalidad acotada: nunca incluyen rutas, ids de paquete, feeds ni principals.

Alertas sugeridas: `onepack_data_dir_free_bytes` por debajo del 10 %; aumento de `onepack_errors_total{code=~"STORAGE_FULL|INTERNAL"}`; `onepack_maintenance == 1` durante más de una hora (un backup interrumpido); `/readyz` distinto de 200.

## Logs

A stderr (journald con systemd). `--log-format json` (`ONEPACK_LOG_FORMAT=json`) escribe una línea JSON por evento; el contenedor y el `.env` de ejemplo lo activan. El nivel se controla con `RUST_LOG` (`info` por defecto; `tower_http=debug` añade una línea por petición).

Cada petición lleva un span `request` con `method`, `uri` y `request_id`. El mismo `request_id` va en la cabecera `X-Request-Id` y en los errores de `/api/v1`, así que el id que reporta un usuario se busca directamente en el log:

```bash
journalctl -u onepackd -o cat | grep 3f9c0a1b2c3d4e5f
```

Los logs nunca contienen tokens ni cabeceras de autorización: el CI lo comprueba en los E2E y en el contenedor.

## Modo mantenimiento

`onepackd backup` lo activa solo; también se puede activar a mano para otras tareas (`onepackd maintenance on --reason … | off | status`). Las mutaciones reciben `503 MAINTENANCE` con `Retry-After`, las lecturas siguen y la limpieza de blobs se pospone. Caduca solo (`--duration-secs`), así que un proceso interrumpido no deja el registro bloqueado.

## Caché de búsqueda

La búsqueda y el autocompletado usan un índice en memoria por feed. Los cambios hechos por el propio servidor (publicar, unlist/relist, bloquear) lo invalidan al momento. Los hechos por otro proceso sobre el mismo directorio de datos (p. ej. `onepackd package block` sin pasar por la API) aparecen en la búsqueda en 5 s como mucho. La descarga de una versión bloqueada se rechaza siempre al momento.
