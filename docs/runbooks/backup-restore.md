# Backup y restauración

> Fase 7 ([ADR-017](../../roadmap/adr-mvp.md#adr-017)).
> Última actualización: 2026-10-04

## Qué incluye

`onepackd backup` produce un directorio con:

- `metadata.sqlite`: copia consistente de la base (`VACUUM INTO`, dentro de una transacción de lectura);
- `blobs/sha256/…`: los blobs que referencia esa copia, verificados contra su hash al copiarlos;
- `manifest.json`: versión del formato y de onepackd, versión del esquema, número de feeds y versiones, y el SHA-256 y tamaño de la base y de cada blob.

**No incluye** la configuración del servicio (`/etc/onepack/onepackd.env` o las variables del contenedor), los certificados TLS ni la credencial administrativa inicial. Guárdalos aparte; los tokens emitidos sí sobreviven, porque la base guarda sus verificadores.

## Hacer un backup

Con el servidor en marcha:

```bash
sudo -u onepack onepackd backup --data-dir /var/lib/onepack --output /var/backups/onepack/$(date +%Y%m%d-%H%M%S)
```

Mientras dura, el registro está en **modo mantenimiento**:

- las lecturas (restore, búsqueda, descargas) siguen funcionando;
- las mutaciones (publicar, unlist, bloquear, API administrativa) reciben `503 MAINTENANCE` con `Retry-After: 30`. `dotnet nuget push` no reintenta: un pipeline que publique durante el backup falla y hay que relanzarlo. Programa los backups fuera de las horas de publicación;
- la limpieza de blobs huérfanos se pospone.

El mantenimiento termina al acabar el backup, también si falla. Si el proceso muere a mitad, caduca solo (`--maintenance-timeout-secs`, 1 h por defecto) o se termina a mano:

```bash
sudo -u onepack onepackd maintenance status --data-dir /var/lib/onepack
sudo -u onepack onepackd maintenance off --data-dir /var/lib/onepack
```

`backup` falla, sin dejar nada a medias, si el destino no está vacío, si ya hay otro mantenimiento o si un blob de origen no coincide con su hash (en ese caso, ejecuta `onepackd check`).

Copia el directorio resultante fuera del servidor (otro disco, otra máquina, almacenamiento de objetos). Un `tar` basta: el manifiesto permite verificarlo después.

## Restaurar

En un directorio de datos **vacío o inexistente**, con el servicio parado:

```bash
sudo systemctl stop onepackd
sudo -u onepack onepackd restore --from /ruta/al/backup --data-dir /var/lib/onepack
sudo -u onepack onepackd check --data-dir /var/lib/onepack
sudo systemctl start onepackd
```

`restore`:

1. verifica el manifiesto, la base y cada blob contra sus hashes, **antes** de copiar nada;
2. copia y vuelve a verificar;
3. si el backup es de una versión anterior, aplica las migraciones (con su backup previo, [ADR-020](../../roadmap/adr-mvp.md#adr-020));
4. comprueba la integridad del resultado.

Si algo falla, deja el directorio vacío. Rechaza un backup de un esquema más nuevo que el binario: actualiza onepackd primero.

Si el servidor restaurado tiene otra URL pública, cambia `ONEPACK_PUBLIC_URL`: las URLs del protocolo se generan siempre a partir de ella ([ADR-018](../../roadmap/adr-mvp.md#adr-018)).

## Comprobar la integridad

```bash
sudo -u onepack onepackd check --data-dir /var/lib/onepack          # lee y verifica cada blob
sudo -u onepack onepackd check --data-dir /var/lib/onepack --quick  # solo existencia y tamaño
```

Informa de:

- blobs **faltantes**: esas versiones no se pueden descargar;
- blobs **dañados**: tamaño o hash distinto;
- huérfanos: archivos que ninguna versión referencia y que la limpieza elimina tras el periodo de gracia.

Sale con código distinto de 0 si hay faltantes o dañados. Las versiones afectadas aparecen como `feed/Id@versión`: restaura sus blobs desde un backup copiando los archivos `blobs/sha256/…` correspondientes.
