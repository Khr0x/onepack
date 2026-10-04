# onepack
One binary, one command, your entire private registry

Registro privado de paquetes NuGet, self-hosted y operado desde CLI.

> Estado: en desarrollo, Fase 0 del [roadmap](roadmap/roadmap-mvp.md). No apto para uso todavía.

## Documentación

- [Propuesta del MVP](roadmap/mvp-registro-privado-paquetes-nuget.md)
- [Roadmap por fases](roadmap/roadmap-mvp.md)
- [Decisiones de arquitectura (ADR)](roadmap/adr-mvp.md)
- [Convenciones](docs/conventions.md)

## Desarrollo

Requisitos: `rustup` (la versión de Rust se toma de `rust-toolchain.toml`) y SDK .NET 8 para las pruebas de conformidad.

```bash
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check-crate-deps.sh
cargo deny check
```

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
