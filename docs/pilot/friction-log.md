# Registro de fricciones del piloto

> Fase 8. Ver [el plan del piloto](README.md) para qué registrar y cómo clasificar.

## Datos del piloto

| Campo | Valor |
|---|---|
| Equipo | _pendiente_ |
| Persona que opera el servidor | _pendiente_ |
| Periodo | _pendiente_ (inicio – fin) |
| Entorno del servidor | _pendiente_ (systemd / contenedor, SO, arquitectura, reverse proxy) |
| Versión de onepack | _pendiente_ |
| CI usado | _pendiente_ |
| Proyectos y librerías | _pendiente_ |

## Fricciones

Clases: **B** = bloqueante MVP, **P** = post-MVP. Estado: abierta, corregida (con enlace al cambio), descartada (con motivo).

| ID | Fecha | Quién / dónde | Qué intentaba | Qué pasó | Tiempo perdido | Clase | Estado |
|---|---|---|---|---|---|---|---|
| F-001 | 2026-10-04 | Prueba decisiva automatizada | Restaurar desde CI con un token revocado | `dotnet restore` solo decía `NU1301: no se puede cargar el índice de servicio`, sin mencionar la credencial. El servidor sí registraba `reason="revoked"`. | — | B | Corregida: `onepack exec` comprueba la credencial antes de lanzar el comando y falla con salida 3 y una acción clara ([cli](../cli.md#nugetconfig-y-credenciales-de-nuget)). |

<!-- Plantilla de fila:
| F-00N | AAAA-MM-DD | persona, máquina o pipeline | | | | B/P | abierta |
-->

## Ejercicio de incidente

Ver [el runbook](../runbooks/incident-drill.md). Una fila por incidente.

| Incidente | Fecha | Tiempo hasta resolverlo | ¿Bastó el runbook? | Desviaciones |
|---|---|---|---|---|
| 1. Credencial de CI filtrada | | | | |
| 2. Versión vulnerable | | | | |
| 3. Pérdida del servidor | | | | |

## Retrospectiva

_Pendiente: respuestas resumidas a las cuatro preguntas del [plan](README.md#retrospectiva)._
