#!/usr/bin/env bash
# systemd unit (Phase 7): installs onepackd as a hardened service, kills it with SIGKILL and
# checks that systemd restarts it and it returns to `ready` without intervention. Requires Linux with
# systemd and sudo (e.g. a GitHub Actions runner).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
port=8080
fail() { echo "SYSTEMD FAILED: $*" >&2; sudo journalctl -u onepackd --no-pager -n 50 >&2 || true; exit 1; }
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

step "Installing binary, user, configuration and unit"
cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server
sudo install -m 0755 "$root/target/debug/onepackd" /usr/local/bin/onepackd
id onepack >/dev/null 2>&1 || sudo useradd --system --no-create-home --shell /usr/sbin/nologin onepack
sudo install -d -m 0750 -o root -g onepack /etc/onepack
sed "s|^ONEPACK_PUBLIC_URL=.*|ONEPACK_PUBLIC_URL=http://127.0.0.1:$port|" "$root/packaging/systemd/onepackd.env" \
  | sudo install -m 0640 -o root -g onepack /dev/stdin /etc/onepack/onepackd.env
sudo install -m 0644 "$root/packaging/systemd/onepackd.service" /etc/systemd/system/onepackd.service
sudo systemd-analyze verify /etc/systemd/system/onepackd.service || fail "the unit is not valid"

step "Initializing /var/lib/onepack as the service user"
sudo rm -rf /var/lib/onepack
sudo install -d -m 0700 -o onepack -g onepack /var/lib/onepack
sudo -u onepack /usr/local/bin/onepackd init --data-dir /var/lib/onepack >/dev/null

step "Starting the service"
sudo systemctl daemon-reload
sudo systemctl start onepackd
ready || fail "did not become ready"
[ "$(sudo stat -c '%a %U' /var/lib/onepack)" = "700 onepack" ] || fail "data directory permissions"
curl -fs "http://127.0.0.1:9464/metrics" | grep -q onepack_build_info || fail "no metrics on the operations listener"

step "Killing the process with SIGKILL"
before=$(systemctl show -p MainPID --value onepackd)
sudo kill -9 "$before"
sleep 1
for _ in $(seq 1 50); do
  after=$(systemctl show -p MainPID --value onepackd)
  [ "$after" != 0 ] && [ "$after" != "$before" ] && ready && break
  sleep 0.2
done
[ "$(systemctl show -p NRestarts --value onepackd)" -ge 1 ] || fail "systemd did not restart it"
ready || fail "it did not return to ready after the restart"
echo "    restarted: PID $before -> $(systemctl show -p MainPID --value onepackd)"

step "JSON logs in journald and graceful shutdown"
logs=$(sudo journalctl -u onepackd --no-pager -o cat -n 5)
grep -q '^{' <<<"$logs" || fail "the logs are not JSON"
sudo systemctl stop onepackd
[ "$(systemctl show -p Result --value onepackd)" = success ] || fail "the shutdown was not clean"

step "Exposure according to systemd-analyze security"
systemd-analyze security onepackd.service 2>/dev/null | tail -1 || true
echo "SYSTEMD OK"
