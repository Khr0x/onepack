# Piloto

> Fase 8. Plan para validar el MVP con un equipo real antes de etiquetar `v0.1.0`.
> Última actualización: 2026-10-04

El piloto responde una pregunta: **¿un equipo que no escribió onepack puede instalarlo, usarlo en su trabajo diario y recuperarse de un incidente solo con la documentación?** Todo lo que lo impida es un bloqueante del MVP.

La [prueba decisiva](../../roadmap/roadmap-mvp.md#prueba-decisiva-del-mvp) ya se ejecuta automáticamente en cada cambio ([`scripts/e2e-decisive.sh`](../../scripts/e2e-decisive.sh)). El piloto comprueba lo que un script no puede: que las personas la completan sin ayuda, en su infraestructura y con sus proyectos.

## Equipo piloto

Requisitos:

- Al menos una librería .NET propia que hoy se comparta por otra vía (carpeta, copia, otro registro).
- Al menos un pipeline de CI que restaure esa librería.
- Una persona que opere el servidor (instalar, hacer backups) y que **no** haya escrito el código de onepack.
- Un servidor Linux o un host de contenedores ([requisitos](../runbooks/install.md)).

Anota en [el registro](friction-log.md#datos-del-piloto) quién participa, el periodo acordado y el entorno.

## Calendario

| Semana | Qué | Salida |
|---|---|---|
| 0 | Preparación: release candidata (ver [release](../runbooks/release.md)), reunión de arranque de 30 min. | Binarios firmados y verificados disponibles. |
| 1 | **Instalación solo con la documentación**: [instalación](../runbooks/install.md) y [guía de cero a restore en CI](../guide-zero-to-ci.md). Nadie del proyecto ayuda salvo que se bloqueen; si pasa, es una fricción. | Servidor en marcha, feed creado, librería publicada desde CI y restaurada en otro pipeline. |
| 1 | Verificación con IDE: Visual Studio y Rider ([pasos](../compatibility.md#verificación-manual-ide)). | Filas de IDE completadas en `docs/compatibility.md`. |
| 1–4 | **Uso habitual**: publicar versiones nuevas desde CI, restaurar en builds y estaciones, rotar algún token. Backup diario programado ([backup](../runbooks/backup-restore.md)). | Fricciones registradas sobre la marcha. |
| 2–3 | **Ejercicio de incidente** ([runbook](../runbooks/incident-drill.md)): revocar la credencial de CI, bloquear una versión y restaurar desde un backup, con cronómetro. | Tiempos y desviaciones anotados en el registro. |
| final | Retrospectiva (ver abajo) y clasificación final de las fricciones. | Lista de bloqueantes MVP cerrada o con fecha. |

Periodo recomendado: 2–4 semanas. Menos de dos semanas no cubre suficientes ciclos de publicación ni un backup real restaurado.

## Qué registrar

Todo en [friction-log.md](friction-log.md). Una fricción es cualquier momento en que alguien:

- tuvo que preguntar algo que la documentación debería haber respondido;
- vio un error que no explicaba qué hacer;
- tuvo que editar un archivo a mano, abrir la base de datos o leer el código;
- perdió más de 10 minutos en un paso;
- encontró un comportamiento distinto al documentado.

No hace falta proponer la solución: basta con qué intentaba, qué pasó y cuánto tiempo costó.

## Clasificación

| Clase | Criterio | Qué pasa |
|---|---|---|
| **Bloqueante MVP** | Pérdida o corrupción de datos; acceso entre feeds; la prueba decisiva no se puede completar sin ayuda, sin UI o sin tocar la base de datos; un error de seguridad. | Se corrige antes de `v0.1.0`. |
| **Post-MVP** | Todo lo demás: comodidad, rendimiento dentro de objetivos, funcionalidades nuevas. | Va a la retrospectiva y al orden de evolución. |

## Criterios de salida

El piloto termina con éxito cuando, además de los [criterios de cierre del MVP](../../roadmap/roadmap-mvp.md#criterios-de-cierre-del-mvp):

- el equipo instaló y publicó sin ayuda directa (o cada ayuda está registrada y corregida en la documentación);
- el ejercicio de incidente se completó y los tres incidentes se resolvieron con los runbooks;
- hubo al menos un backup programado restaurado en otra máquina;
- no queda ninguna fricción **bloqueante MVP** abierta.

## Retrospectiva

Una sesión de 45 minutos al final, con estas preguntas, y las respuestas resumidas en el registro:

1. ¿Qué haríais distinto si volvierais a instalarlo mañana?
2. ¿Qué paso de la documentación sobraba o faltaba?
3. ¿Qué os haría dejar de usarlo?
4. De la [lista fuera del MVP](../../roadmap/roadmap-mvp.md#fuera-del-mvp-referencia) (credential provider, S3, proxy de nuget.org, npm…), ¿qué echasteis en falta primero?

La respuesta a la 4 actualiza el orden de evolución en el roadmap.
