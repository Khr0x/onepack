#!/usr/bin/env python3
"""Benchmark reproducible de onepackd (Fase 7).

Mide, con binarios release:
  - arranque: desde que se lanza el proceso hasta que /readyz responde 200;
  - RSS en reposo;
  - latencia de metadatos (service index, flat container, registro, búsqueda) con un feed
    de 200 paquetes x 5 versiones y 16 clientes concurrentes (onepack-bench);
  - memoria durante la subida y la descarga de un paquete de 200 MiB.

Modos:
  scripts/bench.py             onepackd nativo en esta máquina.
  scripts/bench.py --docker    onepackd en la imagen de packaging/container, limitado a
                               2 vCPU y 1 GiB (perfil objetivo). La memoria se lee del cgroup
                               (memoria anónima, sin caché de páginas) cuando es posible.

Escribe el informe en Markdown en stdout y los datos en JSON en --out (bench-results.json).
"""

import argparse
import json
import os
import platform
import random
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PORT = 18090
IMAGE = "onepackd:bench"
PACKAGES, VERSIONS = 200, 5
LARGE_MIB = 200


def run(*cmd, **kw):
    return subprocess.run(cmd, check=True, text=True, capture_output=True, **kw).stdout


def http_status(url, token=None):
    req = urllib.request.Request(url, headers={"X-NuGet-ApiKey": token} if token else {})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            r.read()
            return r.status
    except urllib.error.HTTPError as e:
        return e.code
    except OSError:
        return 0


def wait_ready(base, timeout=30):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if http_status(f"{base}/readyz") == 200:
            return
        time.sleep(0.005)
    raise SystemExit("onepackd no llegó a ready")


class Native:
    label = "nativo"

    def __init__(self, work):
        self.work, self.data, self.proc = work, os.path.join(work, "data"), None
        self.bin = os.path.join(ROOT, "target/release/onepackd")

    def init(self):
        run(self.bin, "init", "--data-dir", self.data)
        return open(os.path.join(self.data, "initial-admin-token")).read().strip()

    def start(self):
        t0 = time.monotonic()
        self.proc = subprocess.Popen(
            [self.bin, "serve", "--data-dir", self.data, "--listen", f"127.0.0.1:{PORT}",
             "--public-url", f"http://127.0.0.1:{PORT}", "--max-package-size-mib", str(LARGE_MIB + 100),
             "--min-free-space-mib", "0", "--principal-rate-limit", "0", "--ip-rate-limit", "0"],
            stdout=subprocess.DEVNULL, stderr=open(os.path.join(self.work, "server.log"), "w"))
        wait_ready(f"http://127.0.0.1:{PORT}")
        return time.monotonic() - t0

    def memory_mib(self):
        out = run("ps", "-o", "rss=", "-p", str(self.proc.pid)).strip()
        return int(out) / 1024

    def stop(self):
        self.proc.terminate()
        self.proc.wait(timeout=30)


class Docker:
    label = "contenedor (2 vCPU, 1 GiB)"

    def __init__(self, work):
        self.work, self.volume, self.name = work, "onepack-bench", "onepack-bench"
        self.cgroup = None

    def init(self):
        subprocess.run(["docker", "rm", "-f", self.name], capture_output=True)
        subprocess.run(["docker", "volume", "rm", self.volume], capture_output=True)
        run("docker", "volume", "create", self.volume)
        run("docker", "run", "--rm", "-v", f"{self.volume}:/data", IMAGE, "init")
        return run("docker", "run", "--rm", "-v", f"{self.volume}:/data", "alpine:3",
                   "cat", "/data/initial-admin-token").strip()

    def start(self):
        run("docker", "create", "--name", self.name, "--cpus", "2", "--memory", "1g",
            "-p", f"127.0.0.1:{PORT}:8080", "-v", f"{self.volume}:/data",
            "-e", f"ONEPACK_PUBLIC_URL=http://127.0.0.1:{PORT}",
            "-e", f"ONEPACK_MAX_PACKAGE_SIZE_MIB={LARGE_MIB + 100}", "-e", "ONEPACK_MIN_FREE_SPACE_MIB=0",
            "-e", "ONEPACK_PRINCIPAL_RATE_LIMIT=0", "-e", "ONEPACK_IP_RATE_LIMIT=0", IMAGE)
        t0 = time.monotonic()
        run("docker", "start", self.name)
        wait_ready(f"http://127.0.0.1:{PORT}")
        elapsed = time.monotonic() - t0
        cid = run("docker", "inspect", "--format", "{{.Id}}", self.name).strip()
        for path in (f"/sys/fs/cgroup/system.slice/docker-{cid}.scope/memory.stat",
                     f"/sys/fs/cgroup/docker/{cid}/memory.stat"):
            if os.path.exists(path):
                self.cgroup = path
        return elapsed

    def memory_mib(self):
        if self.cgroup:
            for line in open(self.cgroup):
                key, value = line.split()
                if key == "anon":
                    return int(value) / (1 << 20)
        usage = run("docker", "stats", "--no-stream", "--format", "{{.MemUsage}}", self.name)
        value = usage.split("/")[0].strip()
        units = {"KiB": 1 / 1024, "MiB": 1, "GiB": 1024, "B": 1 / (1 << 20)}
        for unit, factor in units.items():
            if value.endswith(unit):
                return float(value[: -len(unit)]) * factor
        return float("nan")

    def stop(self):
        run("docker", "stop", "-t", "10", self.name)
        subprocess.run(["docker", "rm", "-f", self.name], capture_output=True)
        subprocess.run(["docker", "volume", "rm", self.volume], capture_output=True)


class Tee:
    """Escribe el informe también en el resumen del job de GitHub Actions."""

    def __init__(self, *streams):
        self.streams = streams

    def write(self, data):
        for s in self.streams:
            s.write(data)

    def flush(self):
        for s in self.streams:
            s.flush()


def nupkg(path, pkg_id, version, payload):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as z:
        z.writestr(f"{pkg_id}.nuspec",
                   f"<package><metadata><id>{pkg_id}</id><version>{version}</version>"
                   f"<description>benchmark {pkg_id}</description><authors>bench</authors>"
                   f"<tags>bench onepack</tags></metadata></package>")
        z.writestr(f"lib/net8.0/{pkg_id}.dll", payload)


def sample_peak(server, stop, peak):
    while not stop.is_set():
        try:
            peak[0] = max(peak[0], server.memory_mib())
        except Exception:
            pass
        time.sleep(0.05)


def measure_peak(server, action):
    """Memoria máxima durante `action` y la de justo antes, para aislar el coste de la
    transferencia de lo que ya retenía el proceso."""
    before = server.memory_mib()
    stop, peak = threading.Event(), [before]
    t = threading.Thread(target=sample_peak, args=(server, stop, peak))
    t.start()
    t0 = time.monotonic()
    try:
        action()
    finally:
        stop.set()
        t.join()
    return before, peak[0], time.monotonic() - t0


def main():
    global LARGE_MIB
    ap = argparse.ArgumentParser()
    ap.add_argument("--docker", action="store_true")
    ap.add_argument("--seconds", type=int, default=15)
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--large-mib", type=int, default=LARGE_MIB)
    ap.add_argument("--out", default="bench-results.json")
    args = ap.parse_args()
    LARGE_MIB = args.large_mib
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        sys.stdout = Tee(sys.stdout, open(os.environ["GITHUB_STEP_SUMMARY"], "a"))

    print("==> compilando (release)", file=sys.stderr)
    run("cargo", "build", "--release", "--locked", "-p", "onepack-server", "-p", "onepack-cli",
        "-p", "onepack-bench", cwd=ROOT)
    if args.docker:
        print("==> construyendo la imagen", file=sys.stderr)
        run("docker", "build", "-q", "-f", "packaging/container/Containerfile", "-t", IMAGE, ".",
            cwd=ROOT)

    work = tempfile.mkdtemp(prefix="onepack-bench-")
    server = Docker(work) if args.docker else Native(work)
    base = f"http://127.0.0.1:{PORT}"
    onepack = os.path.join(ROOT, "target/release/onepack")
    results = {"large_mib": LARGE_MIB, "mode": server.label, "host": f"{platform.system()} {platform.machine()}",
               "cpus_host": os.cpu_count()}
    try:
        token = server.init()
        env = dict(os.environ, ONEPACK_TOKEN=token, ONEPACK_CONFIG_DIR=os.path.join(work, "cfg"),
                   ONEPACK_KEYRING="off", ONEPACK_NO_INPUT="1")
        cli = lambda *a: run(onepack, "--url", base, *a, env=env)  # noqa: E731

        print("==> arranque", file=sys.stderr)
        results["startup_s"] = server.start()
        time.sleep(3)
        results["idle_rss_mib"] = server.memory_mib()

        print(f"==> publicando {PACKAGES * VERSIONS} versiones", file=sys.stderr)
        cli("feed", "create", "bench")
        pkgs = os.path.join(work, "pkgs")
        os.makedirs(pkgs)
        rnd = random.Random(42)
        files = []
        for i in range(PACKAGES):
            for v in range(VERSIONS):
                f = os.path.join(pkgs, f"Bench.Pkg{i}.1.{v}.0.nupkg")
                nupkg(f, f"Bench.Pkg{i}", f"1.{v}.0", rnd.randbytes(20 * 1024))
                files.append(f)
        t0 = time.monotonic()
        for chunk in range(0, len(files), 100):
            cli("package", "push", "--feed", "bench", *files[chunk:chunk + 100])
        results["publish_small_per_s"] = len(files) / (time.monotonic() - t0)
        results["loaded_rss_mib"] = server.memory_mib()

        print("==> latencia de metadatos", file=sys.stderr)
        bench = run(os.path.join(ROOT, "target/release/onepack-bench"), base, "bench", token,
                    "Bench.Pkg0", str(args.concurrency), str(args.seconds))
        results["metadata"] = json.loads(bench)

        print(f"==> paquete de {LARGE_MIB} MiB", file=sys.stderr)
        large = os.path.join(work, "Bench.Large.1.0.0.nupkg")
        nupkg(large, "Bench.Large", "1.0.0", os.urandom(LARGE_MIB << 20))
        before, peak, secs = measure_peak(server, lambda: cli("--timeout", "600", "package", "push",
                                                               "--feed", "bench", large))
        results["upload_large"] = {"before_mib": before, "peak_mib": peak, "seconds": secs}
        url = f"{base}/nuget/bench/v3/flat/bench.large/1.0.0/bench.large.1.0.0.nupkg"

        def download():
            req = urllib.request.Request(url, headers={"X-NuGet-ApiKey": token})
            with urllib.request.urlopen(req, timeout=600) as r:
                while r.read(1 << 20):
                    pass

        before, peak, secs = measure_peak(server, download)
        results["download_large"] = {"before_mib": before, "peak_mib": peak, "seconds": secs}
    finally:
        try:
            server.stop()
        finally:
            shutil.rmtree(work, ignore_errors=True)

    with open(args.out, "w") as f:
        json.dump(results, f, indent=2)
    report(results)


def report(r):
    m = r["metadata"]
    ok = lambda cond: "✅" if cond else "❌"  # noqa: E731
    print(f"### Benchmark onepackd — {r['mode']}\n")
    print(f"Host: {r['host']}, {r['cpus_host']} CPU lógicas.\n")
    print("| Medida | Resultado | Objetivo | |\n|---|---|---|---|")
    print(f"| Arranque hasta `ready` | {r['startup_s']:.2f} s | < 2 s | {ok(r['startup_s'] < 2)} |")
    print(f"| RSS en reposo | {r['idle_rss_mib']:.1f} MiB | < 100 MiB | {ok(r['idle_rss_mib'] < 100)} |")
    print(f"| p95 metadatos ({m['concurrency']} clientes) | {m['all']['p95_ms']:.1f} ms | < 50 ms | "
          f"{ok(m['all']['p95_ms'] < 50)} |")
    for kind in ("upload_large", "download_large"):
        label = "Subida" if kind == "upload_large" else "Descarga"
        t = r[kind]
        growth = t["peak_mib"] - t["before_mib"]
        print(f"| {label} de {r['large_mib']} MiB: memoria adicional | +{growth:.1f} MiB "
              f"(de {t['before_mib']:.1f} a {t['peak_mib']:.1f} MiB, {t['seconds']:.1f} s) | "
              f"acotada: < 100 MiB, sin depender del tamaño | {ok(growth < 100)} |")
    print(f"\nCon {PACKAGES * VERSIONS} versiones cargadas: RSS {r['loaded_rss_mib']:.1f} MiB; "
          f"{m['requests_per_second']:.0f} peticiones/s de metadatos, {m['errors']} errores; "
          f"publicación de paquetes pequeños: {r['publish_small_per_s']:.0f}/s.\n")
    print("| Recurso | p50 | p95 | p99 |\n|---|---|---|---|")
    for name, s in m["by_resource"].items():
        print(f"| {name} | {s['p50_ms']:.1f} ms | {s['p95_ms']:.1f} ms | {s['p99_ms']:.1f} ms |")


if __name__ == "__main__":
    main()
