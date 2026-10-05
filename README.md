# onepack
One binary, one command, your entire private registry

Registro privado de paquetes NuGet, self-hosted y operado desde CLI.

> Estado: en desarrollo (ver el [roadmap](roadmap/roadmap-mvp.md)). No apto para uso todavía.

## Documentación

- [Propuesta del MVP](roadmap/mvp-registro-privado-paquetes-nuget.md)
- [Roadmap por fases](roadmap/roadmap-mvp.md)
- [Decisiones de arquitectura (ADR)](roadmap/adr-mvp.md)
- [Convenciones](docs/conventions.md)
- [Clientes comprobados](docs/compatibility.md)
- [Seguridad: límites, cuotas y bloqueo de versiones](docs/security.md)
- [Guía: de cero a restore en CI](docs/guide-zero-to-ci.md)
- [CLI `onepack`](docs/cli.md) y [API `/api/v1`](docs/api.md)
- Operación: [instalación](docs/runbooks/install.md), [backup y restauración](docs/runbooks/backup-restore.md), [actualización](docs/runbooks/upgrade.md), [ejercicio de incidente](docs/runbooks/incident-drill.md), [release](docs/runbooks/release.md), [observabilidad](docs/operations.md) y [rendimiento](docs/performance.md)
- [Piloto](docs/pilot/README.md) y [registro de fricciones](docs/pilot/friction-log.md)

## Desarrollo

Requisitos: `rustup` (la versión de Rust se toma de `rust-toolchain.toml`) y SDK .NET 8 para las pruebas de conformidad.

```bash
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-features
./scripts/check-crate-deps.sh
cargo deny check
```

## Ejecutar el servidor

```bash
cargo run -p onepack-server -- init --data-dir ./data
cargo run -p onepack-server -- feed create internal --data-dir ./data
cargo run -p onepack-server -- serve --data-dir ./data --public-url http://127.0.0.1:8080
```

`init` escribe la credencial administrativa inicial en `./data/initial-admin-token`. El resto se opera con el CLI:

```bash
cargo run -p onepack-cli -- context add local --url http://127.0.0.1:8080
cargo run -p onepack-cli -- login --token-stdin < ./data/initial-admin-token
cargo run -p onepack-cli -- feed list
```

La [guía de cero a restore en CI](docs/guide-zero-to-ci.md) recorre el flujo completo: feed, cuenta de CI, token, permisos, publicación, `NuGet.Config` y restore. El feed queda en `http://127.0.0.1:8080/nuget/<feed>/v3/index.json`. Usa HTTPS (directo o con reverse proxy) fuera de local.

Los límites de inspección, las cuotas por feed y el bloqueo de versiones (`onepackd feed quota`, `onepackd package block`) se describen en [docs/security.md](docs/security.md). `onepackd serve --help` lista todos los límites.

## Estructura

```text
crates/
  core/         dominio
  nuget/        adaptador NuGet
  storage/      SQLite y blobs
  api-client/   contratos de /api/v1
  server/       binario onepackd
  cli/          binario onepack
migrations/     migraciones SQL
tests/          integration, conformance-dotnet, security, recovery
packaging/      systemd, container
docs/           documentación técnica
```
