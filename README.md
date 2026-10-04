# onepack
One binary, one command, your entire private registry

Registro privado de paquetes NuGet, self-hosted y operado desde CLI.

> Estado: en desarrollo (ver el [roadmap](roadmap/roadmap-mvp.md)). No apto para uso todavía.

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

`init` escribe la credencial administrativa inicial en `./data/initial-admin-token`. Para una cuenta de CI:

```bash
cargo run -p onepack-server -- principal create ci --kind service --data-dir ./data
cargo run -p onepack-server -- grant set --principal ci --feed internal --role publisher --data-dir ./data
cargo run -p onepack-server -- token create --principal ci --data-dir ./data
```

El feed queda en `http://127.0.0.1:8080/nuget/internal/v3/index.json`. NuGet se autentica con Basic (token como contraseña) o `X-NuGet-ApiKey`. Usa HTTPS (directo o con reverse proxy) fuera de local.

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
