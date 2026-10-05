# Prueba de recuperación

> Gate de la Fase 7: backup en un servidor, restauración en **otra infraestructura** y `dotnet restore` de un proyecto contra el servidor restaurado ([ADR-017](../../roadmap/adr-mvp.md#adr-017)).
> Última actualización: 2026-10-04

## Automatizada (CI)

[`scripts/e2e-recovery.sh`](../../scripts/e2e-recovery.sh) separa las dos mitades, y el CI las ejecuta en máquinas y sistemas operativos distintos:

| Job | Máquina | Qué hace |
|---|---|---|
| `Recovery (1/2) backup on Linux` | `ubuntu-24.04` | Publica los fixtures con `dotnet pack` y `onepack package push`, hace el backup **con el servidor en marcha**, comprueba que el mantenimiento termina y que se vuelve a aceptar escrituras, y pasa `onepackd check`. Sube el backup como artefacto. |
| `Recovery (2/2) restore on macOS` | `macos-15` | Descarga el backup y restaura en un directorio vacío (y comprueba que no restaura sobre datos existentes). Pasa `check`, arranca con **otra URL pública** y comprueba que el token de CI del origen sigue funcionando. Hace `onepack nuget init` y `onepack exec -- dotnet restore` con cachés vacías, ejecuta el consumidor y compara byte a byte los paquetes descargados con los publicados en el origen. |

En local, `scripts/e2e-recovery.sh` sin argumentos ejecuta ambas mitades en la misma máquina.

## Manual (infraestructura real)

Para repetirla con servidores propios:

1. **Origen**: con el servicio en marcha, `onepackd backup --data-dir /var/lib/onepack --output /tmp/onepack-backup` y `tar -czf onepack-backup.tar.gz -C /tmp onepack-backup`.
2. Copia el archivo a una **máquina distinta** (otro host, otra red o proveedor), con onepackd instalado ([instalación](install.md)).
3. **Destino**: `tar -xzf onepack-backup.tar.gz`, `onepackd restore --from onepack-backup --data-dir /var/lib/onepack`, `onepackd check --data-dir /var/lib/onepack`. Configura `ONEPACK_PUBLIC_URL` con la URL del destino y arranca el servicio.
4. Desde una estación con un proyecto que dependa de paquetes del registro: `onepack context add destino --url <URL>`, `onepack nuget init --feed <feed> --pattern '<Prefijo>.*'`, vacía las cachés (`dotnet nuget locals all --clear`) y ejecuta `onepack exec --feed <feed> -- dotnet restore`.
5. Anota el resultado abajo.

## Registro

| Fecha | Origen | Destino | Versiones / blobs | Duración backup / restore | `dotnet restore` | Responsable |
|---|---|---|---|---|---|---|
| 2026-10-04 | macOS (local) | macOS (local, otro directorio y URL) | 5 / 5 | < 1 s / < 1 s | ✅ | automatizada |
| | | | | | | |
