# Performance

> Phase 7. Real results: unmet targets are documented, not hidden.
> Last updated: 2026-10-16

## Method

[`scripts/bench.py`](../scripts/bench.py) builds in release mode and measures:

| Measurement | How |
|---|---|
| Startup | From launching `onepackd serve` (or `docker start`) until the first `200` from `/readyz`, probing every 5 ms. |
| Idle RSS | 3 s after startup, with the database already created. |
| Metadata latency | Feed with 200 packages × 5 versions (1,000 versions). [`onepack-bench`](../crates/bench/src/main.rs) launches 16 concurrent clients for 15 s that round-robin service index, flat container, registry index, and search, all authenticated. Measures end to end at the client. |
| Memory during transfers | Upload (`onepack package push`) and download of a large package, sampling memory every 50 ms. Reports growth relative to just before the transfer. |

There are two modes:

- **Native**: `scripts/bench.py`.
- **Target profile**: `scripts/bench.py --docker` runs onepackd in the [image](../packaging/container/Containerfile) with `--cpus 2 --memory 1g`. Memory is read from the cgroup (anonymous memory, no page cache). CI runs it on every push to `main` and manually with *Run workflow* (job *Benchmark*; it does not run on PRs) and publishes the report in the job summary and in the `bench-results` artifact.

`--large-mib N` changes the size of the large package (200 MiB by default).

## Results

### Native, Apple M4 (10 cores, 24 GiB), macOS — 2026-10-04

| Measurement | Result | Target | |
|---|---|---|---|
| Startup to `ready` | 0.02 s | < 2 s | ✅ |
| Idle RSS | 11.4 MiB | < 100 MiB | ✅ |
| p95 metadata (16 clients) | 1.2 ms | < 50 ms | ✅ |
| Extra memory when uploading 200 MiB | +0.9 MiB | bounded | ✅ |
| Extra memory when downloading 200 MiB | +0.1 MiB | bounded | ✅ |

| Resource | p50 | p95 | p99 |
|---|---|---|---|
| service index | 0.6 ms | 0.8 ms | 0.8 ms |
| flat container | 0.9 ms | 1.0 ms | 1.1 ms |
| registry | 0.9 ms | 1.1 ms | 1.3 ms |
| search | 1.0 ms | 1.3 ms | 1.5 ms |

18,209 metadata requests/s with no errors. With 1,000 versions loaded, idle RSS is 15.4 MiB; after the 15 s load, the process retains about 49 MiB (allocator memory and page cache for SQLite read connections). Publishing small packages runs at 78/s, sequentially from a single client.

**Memory does not depend on package size.** Growth during the transfer in separate runs:

| Package | Upload | Download |
|---|---|---|
| 50 MiB | +0.4 MiB | +0.1 MiB |
| 200 MiB | +0.9 MiB | +0.1 MiB |
| 400 MiB | +0.0 MiB | +5.0 MiB |

Upload is written to staging as it arrives and download is streamed from the file; neither goes through memory.

### Container, 2 vCPU / 1 GiB — 2026-10-16

Results from the [*Benchmark* job](https://github.com/Khr0x/onepack/actions/runs/38917726054/job/117029413791), completed successfully in the [main CI](https://github.com/Khr0x/onepack/actions/runs/38917726054) for commit [`88467a94eaee4fc7345bac54fdf6fa4449a913b8`](https://github.com/Khr0x/onepack/commit/88467a94eaee4fc7345bac54fdf6fa4449a913b8). Data is kept in the [`bench-results`](https://github.com/Khr0x/onepack/actions/runs/38917726054/artifacts/12053542020) artifact (`bench-results.json`).

Environment: `ubuntu-24.04`, Linux x86-64, host with 4 logical CPUs and a container limited with `--cpus 2 --memory 1g`. Command run:

```bash
python3 scripts/bench.py --docker --out bench-results.json
```

| Measurement | Result | Target | |
|---|---|---|---|
| Startup to `ready` | 0.16 s | < 2 s | ✅ |
| Idle anonymous memory (cgroup) | 1.2 MiB | < 100 MiB | ✅ |
| p95 metadata (16 clients) | 11.2 ms | < 50 ms | ✅ |
| Extra memory when uploading 200 MiB | +1.0 MiB | < 100 MiB extra | ✅ |
| Extra memory when downloading 200 MiB | +0.0 MiB | < 100 MiB extra | ✅ |

| Resource | p50 | p95 | p99 |
|---|---|---|---|
| service index | 5.7 ms | 9.4 ms | 11.9 ms |
| flat container | 7.4 ms | 11.5 ms | 13.9 ms |
| registry | 7.6 ms | 11.8 ms | 14.6 ms |
| search | 7.3 ms | 11.2 ms | 14.1 ms |

With 1,000 versions loaded: 3.4 MiB of anonymous memory; 2,125 metadata requests/s, with no errors; publishing small packages: 265/s. The 200 MiB upload took 0.9 s and the download 1.9 s. Rounding from the original report is preserved.

All numeric targets for this run were met. The job publishes results but does not enforce the thresholds as a CI success condition. The memory check corresponds to 200 MiB transfers; this run does not compare different sizes to demonstrate independence from package size.

The script disables per-principal and per-IP request limits during the measurement. The report calls memory "RSS", but this profile measures cgroup anonymous memory, without page cache; it is not directly comparable with the process RSS measured in native mode.

The runner has no dedicated SSD or two reserved CPUs: the limit is imposed by the cgroup on a shared VM. It is a reproducible approximation, not a measurement on target hardware or a performance guarantee in other deployments.

## What changed with the measurement

The first measurement did not meet the latency target: **p95 of 95 ms** with 16 clients, and search was even worse (p50 of 82 ms). With a single client, search cost 2.6 ms and the rest 0.2 ms: the problem was contention, not per-request cost. Each search read the 1,000 listed versions from SQLite and regrouped and sorted them again. With several searches at once, throughput dropped from 1,265 to 484 requests/s.

Solution: an in-memory search index per feed ([`search_cache.rs`](../crates/server/src/search_cache.rs)), invalidated on publish, unlist/relist, or block from the server itself, and with a 5 s TTL for changes made by other processes ([operation](operations.md#search-cache)). Result: p95 of 1.2 ms and about 18,000 requests/s. Expanding the SQLite read pool was also tried (8 → 16 → 32 connections): with the cache, latency does not improve and retained memory increases, so it stays at 8.

Known limit: the index grows with the number of listed versions in the feed (one copy of its metadata per prerelease/SemVer 2 filter combination). With feeds of tens of thousands of versions, memory will need to be measured and, if necessary, the cache bounded.
