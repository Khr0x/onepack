# Release

> Fase 8. Cómo se publica una versión de onepack. El workflow es [`release.yml`](../../.github/workflows/release.yml).
> Última actualización: 2026-10-04

## Qué hace el workflow

1. **Build**: binarios de release del servidor (Linux x86-64 y ARM64, musl estático) y del CLI (Linux, macOS x86-64/ARM64, Windows).
2. **Verify**: ejecuta la [prueba decisiva](../../scripts/e2e-decisive.sh) con los binarios de Linux x86-64 recién compilados (no con una build de depuración): instalar, publicar, restaurar desde CI, revocar, bloquear y recuperar desde backup.
3. **Publish**: `SHA256SUMS`, firma *keyless* de Sigstore (`SHA256SUMS.sigstore.json`), verificación de esa firma y, si se lanzó con una etiqueta, una release **en borrador** en GitHub.

Si `verify` falla, no se firma ni se publica nada.

## Ensayo (sin etiqueta)

Antes de la primera etiqueta, y siempre que cambie el workflow:

1. GitHub → *Actions* → *Release* → *Run workflow* sobre `main`.
2. Los binarios llevan la versión `0.0.0-dev.<sha>` y quedan como artefactos del run (`release`), sin release en GitHub.
3. Descarga el artefacto `release` y comprueba la firma igual que en la [instalación](install.md#1-descargar-y-verificar), con `--certificate-identity "https://github.com/Khr0x/onepack/.github/workflows/release.yml@refs/heads/main"`.

## Publicar una versión

1. La rama `main` está en verde en CI.
2. Actualiza `version` en el `Cargo.toml` raíz (por ejemplo, `0.1.0`) y `Cargo.lock` (`cargo check`), en un PR. El workflow rechaza una etiqueta que no coincida con esa versión.
3. Tras el merge, etiqueta y sube:

   ```bash
   git switch main && git pull
   git tag -a v0.1.0 -m "onepack 0.1.0"
   git push origin v0.1.0
   ```

4. Cuando el workflow termine, revisa la release en borrador (archivos, `SHA256SUMS`, bundle de firma), escribe las notas y publícala.
5. Comprueba la instalación desde la release publicada siguiendo [instalación](install.md) en una máquina limpia.

Para repetir la prueba decisiva con binarios descargados: descomprime `onepack` y `onepackd` en un directorio y ejecuta `ONEPACK_BIN_DIR=<dir> ./scripts/e2e-decisive.sh` desde el repositorio (necesita el SDK .NET 8).

## Requisitos antes de `v0.1.0`

- Licencia del proyecto decidida y en `LICENSE` (el workflow empaqueta `README.md`; añade `LICENSE` al paso *Package* cuando exista).
- Un ensayo sin etiqueta en verde.
