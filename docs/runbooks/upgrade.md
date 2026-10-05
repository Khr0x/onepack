# Actualización de versión

> Fase 7 ([ADR-020](../../roadmap/adr-mvp.md#adr-020)). Migraciones solo hacia adelante.
> Última actualización: 2026-10-04

1. **Backup** completo ([backup](backup-restore.md)):

   ```bash
   sudo -u onepack onepackd backup --data-dir /var/lib/onepack --output /var/backups/onepack/pre-upgrade
   ```

2. **Instala** el binario nuevo, verificando su firma ([instalación](install.md#1-descargar-y-verificar)).

3. **Comprueba** qué migraciones aplicará, sin cambiar nada:

   ```bash
   sudo -u onepack onepackd migrate --check --data-dir /var/lib/onepack
   ```

4. **Para el servicio, migra y arranca**:

   ```bash
   sudo systemctl stop onepackd
   sudo -u onepack onepackd migrate --data-dir /var/lib/onepack
   sudo systemctl start onepackd
   curl -fs http://127.0.0.1:8080/readyz
   ```

   `migrate` guarda antes una copia de la base en `/var/lib/onepack/backups/`. Un servidor con migraciones pendientes no arranca: indica que hay que ejecutar `migrate`.

5. **Verifica**: `onepackd check --quick` y un `dotnet restore` de un proyecto real.

## Volver atrás

No hay migraciones hacia atrás. Para volver a la versión anterior, instala el binario anterior y restaura el backup del paso 1 en un directorio vacío ([restaurar](backup-restore.md#restaurar)). Una versión anterior se niega a abrir un esquema más nuevo, así que no hay riesgo de corromperlo por error.

## Cómo se prueba

`crates/storage/tests/ops.rs` crea un directorio con el esquema de la versión anterior (todas las migraciones menos la última) y datos, comprueba que `migrate --check` informa de la pendiente y que `migrate` la aplica, conserva los datos y deja el backup previo.
