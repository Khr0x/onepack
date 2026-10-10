#!/usr/bin/env bash
# Unidad systemd (Fase 7): instala onepackd como servicio endurecido, lo mata con SIGKILL y
# comprueba que systemd lo reinicia y vuelve a `ready` sin intervención. Requiere Linux con
# systemd y sudo (p. ej. un runner de GitHub Actions).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
port=8080
fail() { echo "SYSTEMD FALLÓ: $*" >&2; sudo journalctl -u onepackd --no-pager -n 50 >&2 || true; exit 1; }
step() { echo "==> $*"; }
cleanup() {
  sudo systemctl stop onepackd 2>/dev/null || true
  sudo rm -f /etc/systemd/system/onepackd.service
  sudo systemctl daemon-reload || true
}
trap cleanup EXIT

ready() {
  for _ in $(seq 1 100); do
    curl -fs "http://127.0.0.1:$port/readyz" >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  return 1
}

step "Instalando binario, usuario, configuración y unidad"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server
sudo install -m 0755 "$root/target/debug/onepackd" /usr/local/bin/onepackd
id onepack >/dev/null 2>&1 || sudo useradd --system --no-create-home --shell /usr/sbin/nologin onepack
sudo install -d -m 0750 -o root -g onepack /etc/onepack
sed "s|^ONEPACK_PUBLIC_URL=.*|ONEPACK_PUBLIC_URL=http://127.0.0.1:$port|" "$root/packaging/systemd/onepackd.env" \
  | sudo install -m 0640 -o root -g onepack /dev/stdin /etc/onepack/onepackd.env
sudo install -m 0644 "$root/packaging/systemd/onepackd.service" /etc/systemd/system/onepackd.service
sudo systemd-analyze verify /etc/systemd/system/onepackd.service || fail "la unidad no es válida"

step "Inicializando /var/lib/onepack como el usuario del servicio"
sudo rm -rf /var/lib/onepack
sudo install -d -m 0700 -o onepack -g onepack /var/lib/onepack
sudo -u onepack /usr/local/bin/onepackd init --data-dir /var/lib/onepack >/dev/null

step "Arrancando el servicio"
sudo systemctl daemon-reload
sudo systemctl start onepackd
ready || fail "no llegó a ready"
[ "$(sudo stat -c '%a %U' /var/lib/onepack)" = "700 onepack" ] || fail "permisos del directorio de datos"
curl -fs "http://127.0.0.1:9464/metrics" | grep -q onepack_build_info || fail "sin métricas en el listener de operación"

step "Matando el proceso con SIGKILL"
before=$(systemctl show -p MainPID --value onepackd)
sudo kill -9 "$before"
sleep 1
for _ in $(seq 1 50); do
  after=$(systemctl show -p MainPID --value onepackd)
  [ "$after" != 0 ] && [ "$after" != "$before" ] && ready && break
  sleep 0.2
done
[ "$(systemctl show -p NRestarts --value onepackd)" -ge 1 ] || fail "systemd no lo reinició"
ready || fail "no volvió a ready tras el reinicio"
echo "    reiniciado: PID $before -> $(systemctl show -p MainPID --value onepackd)"

step "Logs JSON en journald y parada ordenada"
logs=$(sudo journalctl -u onepackd --no-pager -o cat -n 5)
grep -q '^{' <<<"$logs" || fail "los logs no son JSON"
sudo systemctl stop onepackd
[ "$(systemctl show -p Result --value onepackd)" = success ] || fail "la parada no fue limpia"

step "Exposición según systemd-analyze security"
systemd-analyze security onepackd.service 2>/dev/null | tail -1 || true
echo "SYSTEMD OK"
