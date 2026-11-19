# Operations and observability

> Phase 7. Installation, backup and upgrade: [`docs/runbooks/`](runbooks/).
> Last updated: 2026-10-04

## Probes

| Path | Credential | Response |
|---|---|---|
| `GET /healthz` | no | `200 {"status":"ok"}` while the process is serving. |
| `GET /readyz` | no | `200` if the database responds and the data directory is accessible; `503` otherwise, with details for each check. During a backup it stays at `200` and reports maintenance. |

They are on the main listener (for load balancers and orchestrators) and on the operations listener. They do not reveal registry data. They are the only unauthenticated routes on the main listener.

## Metrics

`--ops-listen 127.0.0.1:9464` (`ONEPACK_OPS_LISTEN`) opens a second listener, without authentication, with `/metrics` in Prometheus format and the probes. Expose it only to an internal network.

| Metric | Type | Labels |
|---|---|---|
| `onepack_http_requests_total` | counter | `surface` (`nuget`, `admin`, `probe`), `method`, `status` |
| `onepack_http_request_duration_seconds` | histogram | `surface` |
| `onepack_errors_total` | counter | `code` (stable codes: `RATE_LIMITED`, `PACKAGE_UNSAFE_PATH`, `FEED_QUOTA_STORAGE`, `MAINTENANCE`…) |
| `onepack_uploads_total` | counter | `outcome` (`published`, `conflict`, `rejected`) |
| `onepack_uploads_in_flight` | gauge | |
| `onepack_maintenance` | gauge | 1 during maintenance |
| `onepack_data_dir_free_bytes` | gauge | |
| `onepack_build_info` | gauge | `version` |

Labels have bounded cardinality: they never include paths, package ids, feeds, or principals.

Suggested alerts: `onepack_data_dir_free_bytes` below 10%; an increase in `onepack_errors_total{code=~"STORAGE_FULL|INTERNAL"}`; `onepack_maintenance == 1` for more than an hour (an interrupted backup); `/readyz` other than 200.

## Logs

To stderr (journald with systemd). `--log-format json` (`ONEPACK_LOG_FORMAT=json`) writes one JSON line per event; the container and the example `.env` enable it. The level is controlled with `RUST_LOG` (`info` by default; `tower_http=debug` adds one line per request).

Every request carries a `request` span with `method`, `uri`, and `request_id`. The same `request_id` goes in the `X-Request-Id` header and in `/api/v1` errors, so the id a user reports can be searched directly in the log:

```bash
journalctl -u onepackd -o cat | grep 3f9c0a1b2c3d4e5f
```

Logs never contain tokens or authorization headers: CI checks it in the E2Es and in the container.

## Maintenance mode

`onepackd backup` enables it automatically; it can also be enabled manually for other tasks (`onepackd maintenance on --reason … | off | status`). Mutations receive `503 MAINTENANCE` with `Retry-After`, reads continue, and blob cleanup is postponed. It expires on its own (`--duration-secs`), so an interrupted process does not leave the registry locked.

## Search cache

Search and autocomplete use an in-memory index per feed. Changes made by the server itself (publish, unlist/relist, block) invalidate it immediately. Changes made by another process on the same data directory (e.g., `onepackd package block` without going through the API) appear in search within at most 5 s. Downloading a blocked version is always rejected immediately.
