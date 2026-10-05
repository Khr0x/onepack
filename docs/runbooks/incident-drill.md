# Ejercicio de incidente

> Fase 8. Tres incidentes en orden, con cronómetro, sobre el servidor del piloto. La versión automatizada de este ejercicio es [`scripts/e2e-decisive.sh`](../../scripts/e2e-decisive.sh) y se ejecuta en cada cambio.
> Última actualización: 2026-10-04

Reglas: solo `onepack`, `onepackd` y la documentación. Sin interfaz web, sin abrir la base de datos y sin ayuda de quien escribió el código. Anota los tiempos y cualquier desviación en [el registro](../pilot/friction-log.md#ejercicio-de-incidente).

Antes de empezar:

- [ ] Hay un pipeline de CI que restaura desde el feed con la cuenta de servicio (por ejemplo, `ci`).
- [ ] Hay un backup reciente ([backup](backup-restore.md)) **copiado fuera del servidor**.
- [ ] Tienes una credencial de administrador en el CLI (`onepack whoami` dice `administrador`).

En los ejemplos, el feed es `internal`, la cuenta de CI es `ci` y la librería es `Hemia.Core`.

## 1. La credencial de CI se ha filtrado

Objetivo: el token filtrado deja de funcionar y el pipeline vuelve a funcionar con uno nuevo.

```bash
onepack token list --principal ci                          # identifica el token (columna ID)
onepack token revoke <ID>                                  # pide confirmación
onepack token create --principal ci --name pipeline --expires-in-days 90
```

1. Guarda el token nuevo en el secreto del pipeline ([guía](../guide-zero-to-ci.md)) **antes** de relanzarlo.
2. Comprueba que el token antiguo ya no sirve: `onepack --token-env OLD whoami` termina con salida 3. Con `onepack exec`, el error dice que la credencial no es válida; con `dotnet restore` directamente, NuGet solo muestra `NU1301` y el servidor registra `autenticación rechazada reason="revoked"`.
3. Relanza el pipeline: debe restaurar.
4. Revisa qué hizo el token mientras estuvo filtrado: `onepack audit list --limit 100` (actor `ci`) y la columna *último uso* de `onepack token list`.

**Hecho cuando:** el pipeline restaura con el token nuevo y `token list` muestra el antiguo como revocado.

## 2. Una versión publicada es vulnerable

Objetivo: nadie puede descargar la versión vulnerable, y quien la tenía ve por qué.

```bash
onepack package block --feed internal Hemia.Core 1.4.0 --reason "CVE-2026-0001"
```

1. En una máquina sin caché (`NUGET_PACKAGES` vacío), un restore que la necesite falla con `410 (PACKAGE_BLOCKED - version blocked by the registry)` ([seguridad](../security.md#bloqueo-de-versiones)). La versión sigue apareciendo como obsoleta en los metadatos.
2. Publica la versión corregida (por ejemplo, `1.4.1`) desde CI y actualiza la referencia del proyecto.
3. Cuando ya no haga falta bloquearla, o si fue un error: `onepack package unblock --feed internal Hemia.Core 1.4.0 --reason "..."`.

Las cachés locales de NuGet que ya tenían la versión **no** se invalidan: el bloqueo impide nuevas descargas. Avisa al equipo para que limpien `~/.nuget/packages/hemia.core/1.4.0` si hace falta.

**Hecho cuando:** un restore limpio no puede obtener la versión bloqueada y `onepack audit list --action package.block` muestra el bloqueo con su autor.

## 3. Se ha perdido el servidor

Objetivo: el servicio vuelve en otra máquina (o en la misma, con el disco vacío) desde el último backup, con la misma URL pública.

Sigue [backup y restauración](backup-restore.md#restaurar). En resumen, en la máquina nueva:

```bash
onepackd restore --from /ruta/al/backup --data-dir /var/lib/onepack
onepackd check --data-dir /var/lib/onepack
sudo systemctl start onepackd        # o el contenedor, con el mismo volumen
curl -fs https://packages.example.com/readyz
```

1. Apunta el DNS o el reverse proxy a la máquina nueva si cambió.
2. Relanza el pipeline de CI **sin cambiar su token**: las credenciales viajan en el backup.
3. Lo publicado después del backup se ha perdido: vuelve a publicarlo desde CI (`onepack package push --skip-existing-identical` evita conflictos con lo que sí estaba).
4. Los cambios de seguridad posteriores al backup también se han perdido: **repite las revocaciones y los bloqueos** que hiciste después del backup (consulta tus notas del incidente; la auditoría restaurada no los incluye).

**Hecho cuando:** `/readyz` responde `ready`, el pipeline restaura y `onepackd check` no informa de errores.

## Después del ejercicio

- Anota en el registro el tiempo de cada incidente y cada paso en el que la documentación no bastó.
- Si se usó un token de administrador para el ejercicio, revócalo.
