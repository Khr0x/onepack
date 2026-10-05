# Empaquetado

| Directorio | Contenido | Documentación |
|---|---|---|
| [`systemd/`](systemd/) | Unidad endurecida y archivo de entorno de ejemplo. | [Instalación con systemd](../docs/runbooks/install.md#2-servicio-systemd) |
| [`container/`](container/) | Imagen sin root sobre distroless, binario estático. | [Contenedor](../docs/runbooks/install.md#3-contenedor) |
| [`proxy/`](proxy/) | Nginx y Caddy con TLS, sin servir blobs directamente. | [Reverse proxy](../docs/runbooks/install.md#4-reverse-proxy-y-tls) |

Los binarios firmados de cada versión los publica el workflow [`release.yml`](../.github/workflows/release.yml). Las pruebas de estos artefactos están en [`tests/ops/`](../tests/ops/) y se ejecutan en el CI (job *systemd, container and reverse proxy examples*).
