#!/usr/bin/env bash
# Valida la sintaxis de los ejemplos de reverse proxy con las imágenes oficiales de Nginx y
# Caddy (Fase 7).
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
certs=$(mktemp -d)
trap 'rm -rf "$certs"' EXIT
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=packages.example.com" \
  -keyout "$certs/privkey.pem" -out "$certs/fullchain.pem" >/dev/null 2>&1

echo "==> nginx -t"
docker run --rm \
  -v "$root/packaging/proxy/nginx.conf:/etc/nginx/conf.d/default.conf:ro" \
  -v "$certs:/etc/ssl/onepack:ro" \
  --add-host onepackd:127.0.0.1 \
  nginx:stable nginx -t

echo "==> caddy validate"
docker run --rm -v "$root/packaging/proxy/Caddyfile:/etc/caddy/Caddyfile:ro" \
  caddy:2 caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
echo "PROXY OK"
