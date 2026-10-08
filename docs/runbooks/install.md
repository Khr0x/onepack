# Instalación

> Fase 7. Servidor en Linux (x86-64 o ARM64) con systemd, o en contenedor. CLI en Linux, macOS y Windows.
> Última actualización: 2026-10-04

## 1. Descargar y verificar

Cada release publica binarios, `SHA256SUMS` y `SHA256SUMS.sigstore.json`: la firma *keyless* de Sigstore, hecha por el workflow de release del repositorio. No hay clave privada que custodiar.

```bash
VERSION=0.1.0            # sin la "v"
TARGET=x86_64-unknown-linux-musl   # o aarch64-unknown-linux-musl
base=https://github.com/Khr0x/onepack/releases/download/v$VERSION
curl -fLO "$base/onepackd-$VERSION-$TARGET.tar.gz"
curl -fLO "$base/SHA256SUMS"
curl -fLO "$base/SHA256SUMS.sigstore.json"

# La firma demuestra que SHA256SUMS lo generó el workflow de release de este repositorio, para esa etiqueta.
cosign verify-blob --bundle SHA256SUMS.sigstore.json \
  --certificate-identity "https://github.com/Khr0x/onepack/.github/workflows/release.yml@refs/tags/v$VERSION" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
sha256sum --check --ignore-missing SHA256SUMS

tar -xzf "onepackd-$VERSION-$TARGET.tar.gz"
sudo install -m 0755 "onepackd-$VERSION-$TARGET/onepackd" /usr/local/bin/onepackd
```

Los binarios de Linux son estáticos (musl): no dependen de la versión de glibc. El CLI (`onepack-…`) se instala igual en cada estación; en Windows viene en un `.zip`.

## 2. Servicio systemd

Archivos: [`packaging/systemd/onepackd.service`](../../packaging/systemd/onepackd.service) y [`packaging/systemd/onepackd.env`](../../packaging/systemd/onepackd.env).

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin onepack
sudo install -d -m 0750 -o root -g onepack /etc/onepack
sudo install -m 0640 -o root -g onepack packaging/systemd/onepackd.env /etc/onepack/onepackd.env
sudo editor /etc/onepack/onepackd.env          # ONEPACK_PUBLIC_URL como mínimo
sudo install -m 0644 packaging/systemd/onepackd.service /etc/systemd/system/

sudo install -d -m 0700 -o onepack -g onepack /var/lib/onepack
sudo -u onepack onepackd init --data-dir /var/lib/onepack
sudo systemctl daemon-reload
sudo systemctl enable --now onepackd
curl -fs http://127.0.0.1:8080/readyz
```

La credencial administrativa inicial queda en `/var/lib/onepack/initial-admin-token`. Guárdala en un gestor de secretos y borra el archivo ([guía](../guide-zero-to-ci.md#3-set-up-the-cli-as-administrator)).

La unidad:

- corre como el usuario `onepack`, sin capacidades ni escalada de privilegios (`NoNewPrivileges`, `CapabilityBoundingSet=`);
- ve el sistema de archivos en solo lectura salvo `/var/lib/onepack` (`ProtectSystem=strict`, `StateDirectory`), sin `/home` ni dispositivos;
- restringe syscalls, familias de sockets, espacios de nombres y memoria ejecutable;
- se reinicia sola tras un fallo (`Restart=on-failure`) y, al pararla, termina las peticiones en curso (SIGTERM).

`systemd-analyze security onepackd` muestra la exposición resultante. El CI instala la unidad, mata el proceso con SIGKILL y comprueba que vuelve a `ready` ([`tests/ops/systemd.sh`](../../tests/ops/systemd.sh)).

## 3. Contenedor

[`packaging/container/Containerfile`](../../packaging/container/Containerfile): binario estático sobre `distroless/static`, usuario 65532 sin privilegios y volumen `/data`.

```bash
docker build -f packaging/container/Containerfile -t onepackd .
docker volume create onepack-data
docker run --rm -v onepack-data:/data onepackd init
docker run -d --name onepackd --restart unless-stopped \
  -p 127.0.0.1:8080:8080 -p 127.0.0.1:9464:9464 \
  -e ONEPACK_PUBLIC_URL=https://packages.example.com \
  -v onepack-data:/data onepackd
```

La imagen escucha en `0.0.0.0:8080` (API y NuGet) y `0.0.0.0:9464` (métricas y sondas): publica ese segundo puerto solo hacia la red interna. Para el backup, ejecuta `onepackd backup` en otro contenedor con el mismo volumen ([backup](backup-restore.md)).

## 4. Reverse proxy y TLS

Ejemplos en [`packaging/proxy/`](../../packaging/proxy/): [Nginx](../../packaging/proxy/nginx.conf) y [Caddy](../../packaging/proxy/Caddyfile). Ambos:

- terminan TLS y reenvían **todo** a onepackd, que autentica y autoriza cada petición. Nunca sirven el directorio de datos directamente ([ADR-018](../../roadmap/adr-mvp.md#adr-018));
- aceptan cuerpos del tamaño máximo de paquete y transmiten subidas y descargas sin buffer;
- asumen `ONEPACK_PUBLIC_URL` igual a la URL pública y `ONEPACK_IP_RATE_LIMIT=0`, porque detrás del proxy todas las peticiones llegan con su IP. El límite por IP se aplica en el proxy (Nginx) o en el firewall.

El CI valida su sintaxis con las imágenes oficiales ([`tests/ops/proxy-configs.sh`](../../tests/ops/proxy-configs.sh)).
