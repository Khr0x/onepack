# Rendimiento

> Fase 7. Resultados reales: los objetivos no cumplidos se documentan, no se ocultan.
> Última actualización: 2026-10-04

## Método

[`scripts/bench.py`](../scripts/bench.py) compila en release y mide:

| Medida | Cómo |
|---|---|
| Arranque | Desde que se lanza `onepackd serve` (o `docker start`) hasta el primer `200` de `/readyz`, sondeando cada 5 ms. |
| RSS en reposo | 3 s después de arrancar, con la base ya creada. |
| Latencia de metadatos | Feed con 200 paquetes × 5 versiones (1.000 versiones). [`onepack-bench`](../crates/bench/src/main.rs) lanza 16 clientes concurrentes durante 15 s que piden en rueda service index, flat container, índice de registro y búsqueda, todo autenticado. Mide de extremo a extremo en el cliente. |
| Memoria en transferencias | Subida (`onepack package push`) y descarga de un paquete grande, muestreando la memoria cada 50 ms. Se informa del crecimiento respecto a justo antes de la transferencia. |

Hay dos modos:

- **Nativo**: `scripts/bench.py`.
- **Perfil objetivo**: `scripts/bench.py --docker` ejecuta onepackd en la [imagen](../packaging/container/Containerfile) con `--cpus 2 --memory 1g`. La memoria se lee del cgroup (memoria anónima, sin caché de páginas). El CI lo ejecuta en cada push a `main` y a mano con *Run workflow* (job *Benchmark*; no corre en los PRs) y publica el informe en el resumen del job y en el artefacto `bench-results`.

`--large-mib N` cambia el tamaño del paquete grande (200 MiB por defecto).

## Resultados

### Nativo, Apple M4 (10 núcleos, 24 GiB), macOS — 2026-10-04

| Medida | Resultado | Objetivo | |
|---|---|---|---|
| Arranque hasta `ready` | 0,02 s | < 2 s | ✅ |
| RSS en reposo | 11,4 MiB | < 100 MiB | ✅ |
| p95 metadatos (16 clientes) | 1,2 ms | < 50 ms | ✅ |
| Memoria adicional al subir 200 MiB | +0,9 MiB | acotada | ✅ |
| Memoria adicional al descargar 200 MiB | +0,1 MiB | acotada | ✅ |

| Recurso | p50 | p95 | p99 |
|---|---|---|---|
| service index | 0,6 ms | 0,8 ms | 0,8 ms |
| flat container | 0,9 ms | 1,0 ms | 1,1 ms |
| registro | 0,9 ms | 1,1 ms | 1,3 ms |
| búsqueda | 1,0 ms | 1,3 ms | 1,5 ms |

18.209 peticiones/s de metadatos sin errores. Con 1.000 versiones cargadas, el RSS en reposo es de 15,4 MiB; tras los 15 s de carga, el proceso retiene unos 49 MiB (memoria del asignador y caché de páginas de las conexiones de lectura de SQLite). La publicación de paquetes pequeños va a 78/s, de forma secuencial desde un solo cliente.

**La memoria no depende del tamaño del paquete.** Crecimiento durante la transferencia en ejecuciones separadas:

| Paquete | Subida | Descarga |
|---|---|---|
| 50 MiB | +0,4 MiB | +0,1 MiB |
| 200 MiB | +0,9 MiB | +0,1 MiB |
| 400 MiB | +0,0 MiB | +5,0 MiB |

La subida se escribe en staging mientras llega y la descarga se transmite desde el archivo; ninguna pasa por memoria.

### Contenedor, 2 vCPU / 1 GiB

Lo mide el job *Benchmark* del CI en `ubuntu-24.04`, con la imagen limitada con `--cpus 2 --memory 1g`. El job pasó en la ejecución que cerró la Fase 7 ([CI run 37247176082](https://github.com/Khr0x/onepack/actions/runs/37247176082)); sus cifras están en el resumen del job y en el artefacto `bench-results`, y se transcribirán aquí en la Fase 8.

| Medida | Resultado | Objetivo |
|---|---|---|
| Arranque hasta `ready` | pendiente del CI | < 2 s |
| RSS en reposo | pendiente del CI | < 100 MiB |
| p95 metadatos | pendiente del CI | < 50 ms |
| Memoria adicional en transferencias | pendiente del CI | acotada |

El runner no es un disco SSD dedicado ni una máquina de 2 vCPU reales: el límite lo pone el cgroup sobre una VM compartida. Es una aproximación reproducible, no una medición en el hardware objetivo.

## Lo que cambió con la medición

La primera medición no cumplía el objetivo de latencia: **p95 de 95 ms** con 16 clientes, y la búsqueda era aún peor (p50 de 82 ms). Con un solo cliente la búsqueda costaba 2,6 ms y el resto 0,2 ms: el problema era contención, no coste por petición. Cada búsqueda leía de SQLite las 1.000 versiones listadas y volvía a agruparlas y ordenarlas. Con varias búsquedas a la vez, el throughput bajaba de 1.265 a 484 peticiones/s.

Solución: un índice de búsqueda en memoria por feed ([`search_cache.rs`](../crates/server/src/search_cache.rs)), invalidado al publicar, hacer unlist/relist o bloquear desde el propio servidor, y con un TTL de 5 s para cambios hechos por otros procesos ([operación](operations.md#caché-de-búsqueda)). Resultado: p95 de 1,2 ms y unas 18.000 peticiones/s. También se probó a ampliar el pool de lectura de SQLite (8 → 16 → 32 conexiones): con la caché no mejora la latencia y aumenta la memoria retenida, así que se mantiene en 8.

Límite conocido: el índice crece con el número de versiones listadas del feed (una copia de sus metadatos por combinación de filtros prerelease/SemVer 2). Con feeds de decenas de miles de versiones habrá que medir la memoria y, si hace falta, acotar la caché.
