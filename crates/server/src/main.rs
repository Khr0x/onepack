use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use onepack_core::{
    FeedName, FeedQuota, Principal, PrincipalKind, PrincipalName, PublishPattern, Role,
};
use onepack_nuget::{InspectionLimits, NuGetVersion, PackageId};
use onepack_server::{Limits, RateLimit, app, backfill_metadata, gc_loop};
use onepack_storage::{Store, VersionChange, migrate};
use tracing_subscriber::EnvFilter;

type CliResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Archivo con la credencial administrativa inicial (permisos 0600).
const INITIAL_TOKEN_FILE: &str = "initial-admin-token";
const INITIAL_TOKEN_TTL_DAYS: i64 = 30;

#[derive(Parser)]
#[command(
    name = "onepackd",
    version,
    about = "Servidor del registro privado onepack"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inicializa un directorio de datos: migra el esquema y genera la credencial
    /// administrativa inicial.
    Init(DataDir),
    /// Crea o actualiza el esquema del directorio de datos (hace backup si ya había datos).
    Migrate(DataDir),
    /// Gestión local de feeds.
    #[command(subcommand)]
    Feed(FeedCommand),
    /// Gestión local de principals (usuarios y cuentas de servicio).
    #[command(subcommand)]
    Principal(PrincipalCommand),
    /// Gestión local de tokens.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Gestión local de permisos por feed.
    #[command(subcommand)]
    Grant(GrantCommand),
    /// Gestión local de versiones publicadas.
    #[command(subcommand)]
    Package(PackageCommand),
    /// Inicia el servidor.
    Serve(ServeArgs),
}

#[derive(Args)]
struct DataDir {
    /// Directorio de datos.
    #[arg(long, env = "ONEPACK_DATA_DIR")]
    data_dir: PathBuf,
}

#[derive(Subcommand)]
enum FeedCommand {
    /// Crea un feed NuGet.
    Create {
        name: String,
        #[command(flatten)]
        data: DataDir,
    },
    /// Lista los feeds.
    List {
        #[command(flatten)]
        data: DataDir,
    },
    /// Muestra el uso y las cuotas de un feed.
    Show {
        name: String,
        #[command(flatten)]
        data: DataDir,
    },
    /// Fija las cuotas de un feed (0 = sin límite). No afecta a lo ya publicado.
    Quota {
        name: String,
        /// Almacenamiento máximo, en MiB (suma de los tamaños de todas las versiones).
        #[arg(long)]
        max_storage_mib: u64,
        /// Número máximo de versiones.
        #[arg(long)]
        max_versions: u64,
        #[command(flatten)]
        data: DataDir,
    },
}

#[derive(Subcommand)]
enum PackageCommand {
    /// Bloquea la descarga de una versión (sigue visible en los metadatos).
    Block(AvailabilityArgs),
    /// Desbloquea una versión.
    Unblock(AvailabilityArgs),
}

#[derive(Args)]
struct AvailabilityArgs {
    #[arg(long)]
    feed: String,
    #[arg(long)]
    id: String,
    #[arg(long)]
    version: String,
    /// Motivo; queda en la auditoría.
    #[arg(long)]
    reason: String,
    #[command(flatten)]
    data: DataDir,
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    User,
    Service,
}

#[derive(Subcommand)]
enum PrincipalCommand {
    /// Crea un usuario o una cuenta de servicio.
    Create {
        name: String,
        #[arg(long, value_enum, default_value = "user")]
        kind: KindArg,
        /// Rol Administrator: gestiona identidades, permisos y todos los feeds.
        #[arg(long)]
        admin: bool,
        #[command(flatten)]
        data: DataDir,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Emite un token. El token se escribe en stdout y no se puede recuperar después.
    Create {
        #[arg(long)]
        principal: String,
        /// Nombre descriptivo (p. ej. "pipeline de pagos").
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(i64).range(1..=3650))]
        expires_in_days: i64,
        #[command(flatten)]
        data: DataDir,
    },
    /// Revoca un token por su id (los 16 caracteres que siguen a `opk_`).
    Revoke {
        id: String,
        #[command(flatten)]
        data: DataDir,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum RoleArg {
    Reader,
    Publisher,
    Maintainer,
}

#[derive(Subcommand)]
enum GrantCommand {
    /// Asigna (o reemplaza) el rol de un principal en un feed.
    Set {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        feed: String,
        #[arg(long, value_enum)]
        role: RoleArg,
        /// Restringe la publicación a ids que coincidan (p. ej. "Hemia.Payments.*"). Repetible.
        #[arg(long = "publish-pattern")]
        publish_patterns: Vec<String>,
        #[command(flatten)]
        data: DataDir,
    },
    /// Quita el acceso de un principal a un feed.
    Remove {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        feed: String,
        #[command(flatten)]
        data: DataDir,
    },
}

#[derive(Args)]
struct ServeArgs {
    #[command(flatten)]
    data: DataDir,
    /// Dirección de escucha.
    #[arg(long, env = "ONEPACK_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// URL pública con la que los clientes acceden al servidor (p. ej. https://packages.example.com).
    #[arg(long, env = "ONEPACK_PUBLIC_URL")]
    public_url: String,
    /// Tamaño máximo de un paquete, en MiB.
    #[arg(long, env = "ONEPACK_MAX_PACKAGE_SIZE_MIB", default_value_t = 100)]
    max_package_size_mib: u64,
    /// Antigüedad mínima, en segundos, de un archivo en staging o de un blob huérfano para
    /// eliminarlo. Debe superar la duración de cualquier subida: un valor bajo puede borrar
    /// subidas en curso. Usa 0 solo en pruebas.
    #[arg(long, env = "ONEPACK_GC_GRACE_SECS", default_value_t = 3600)]
    gc_grace_secs: u64,
    /// Intervalo entre limpiezas, en segundos.
    #[arg(long, env = "ONEPACK_GC_INTERVAL_SECS", default_value_t = 600)]
    gc_interval_secs: u64,
    /// Por debajo de este espacio libre, en MiB, se emite un aviso al arrancar.
    #[arg(long, env = "ONEPACK_MIN_FREE_SPACE_MIB", default_value_t = 1024)]
    min_free_space_mib: u64,
    #[command(flatten)]
    limits: LimitArgs,
}

/// Límites frente a paquetes no confiables y saturación (ADR-014).
#[derive(Args)]
struct LimitArgs {
    /// Número máximo de entradas del ZIP de un paquete.
    #[arg(long, env = "ONEPACK_MAX_ZIP_ENTRIES", default_value_t = 20_000)]
    max_zip_entries: u64,
    /// Tamaño descomprimido máximo de una entrada del paquete, en MiB.
    #[arg(long, env = "ONEPACK_MAX_ENTRY_SIZE_MIB", default_value_t = 512)]
    max_entry_size_mib: u64,
    /// Tamaño descomprimido máximo del paquete completo, en MiB.
    #[arg(
        long,
        env = "ONEPACK_MAX_UNCOMPRESSED_SIZE_MIB",
        default_value_t = 2048
    )]
    max_uncompressed_size_mib: u64,
    /// Tamaño máximo del .nuspec, en KiB.
    #[arg(long, env = "ONEPACK_MAX_NUSPEC_SIZE_KIB", default_value_t = 1024)]
    max_nuspec_size_kib: u64,
    /// Profundidad máxima de anidamiento XML del .nuspec.
    #[arg(long, env = "ONEPACK_MAX_XML_DEPTH", default_value_t = 32)]
    max_xml_depth: usize,
    /// Tiempo máximo de inspección de un paquete, en segundos.
    #[arg(long, env = "ONEPACK_INSPECTION_TIMEOUT_SECS", default_value_t = 30)]
    inspection_timeout_secs: u64,
    /// Inspecciones simultáneas (por defecto, la mitad de los núcleos).
    #[arg(long, env = "ONEPACK_MAX_CONCURRENT_INSPECTIONS")]
    max_concurrent_inspections: Option<usize>,
    /// Subidas simultáneas; el resto recibe 503.
    #[arg(long, env = "ONEPACK_MAX_CONCURRENT_UPLOADS", default_value_t = 8)]
    max_concurrent_uploads: usize,
    /// Tiempo máximo para recibir una subida, en segundos.
    #[arg(long, env = "ONEPACK_UPLOAD_TIMEOUT_SECS", default_value_t = 600)]
    upload_timeout_secs: u64,
    /// Peticiones por segundo sostenidas por principal (0 = sin límite).
    #[arg(long, env = "ONEPACK_PRINCIPAL_RATE_LIMIT", default_value_t = 50)]
    principal_rate_limit: u32,
    /// Ráfaga máxima por principal.
    #[arg(long, env = "ONEPACK_PRINCIPAL_RATE_BURST", default_value_t = 1000)]
    principal_rate_burst: u32,
    /// Peticiones por segundo sostenidas por IP (0 = sin límite). Detrás de un reverse
    /// proxy todas comparten la IP del proxy.
    #[arg(long, env = "ONEPACK_IP_RATE_LIMIT", default_value_t = 100)]
    ip_rate_limit: u32,
    /// Ráfaga máxima por IP.
    #[arg(long, env = "ONEPACK_IP_RATE_BURST", default_value_t = 2000)]
    ip_rate_burst: u32,
}

impl LimitArgs {
    fn limits(&self, max_package_bytes: u64) -> Limits {
        let defaults = Limits::default();
        Limits {
            max_package_bytes,
            inspection: InspectionLimits {
                max_entries: self.max_zip_entries,
                max_entry_bytes: self.max_entry_size_mib << 20,
                max_total_bytes: self.max_uncompressed_size_mib << 20,
                max_nuspec_bytes: self.max_nuspec_size_kib << 10,
                max_xml_depth: self.max_xml_depth,
                timeout: Duration::from_secs(self.inspection_timeout_secs),
            },
            max_concurrent_uploads: self.max_concurrent_uploads,
            max_concurrent_inspections: self
                .max_concurrent_inspections
                .unwrap_or(defaults.max_concurrent_inspections),
            upload_timeout: Duration::from_secs(self.upload_timeout_secs),
            principal_rate: RateLimit::new(self.principal_rate_limit, self.principal_rate_burst),
            ip_rate: RateLimit::new(self.ip_rate_limit, self.ip_rate_burst),
        }
    }
}

/// Actor de auditoría para los comandos locales: quien tiene acceso al directorio de datos.
fn local_actor() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    format!("local:{user}")
}

async fn require_principal(store: &Store, name: &str) -> CliResult<Principal> {
    Ok(store
        .principal(name)
        .await?
        .ok_or_else(|| format!("no existe el principal {name:?}"))?)
}

async fn require_feed(store: &Store, name: &str) -> CliResult<onepack_core::Feed> {
    Ok(store
        .feed(name)
        .await?
        .ok_or_else(|| format!("no existe el feed {name:?}"))?)
}

#[tokio::main]
async fn main() -> CliResult {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        // Sin códigos de color cuando el log va a un archivo o a journald.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match Cli::parse().command {
        Command::Init(DataDir { data_dir }) => init(&data_dir).await?,
        Command::Migrate(DataDir { data_dir }) => {
            let report = migrate(&data_dir).await?;
            match report.backup {
                Some(backup) => println!(
                    "{} migración(es) aplicada(s); backup previo en {}",
                    report.applied,
                    backup.display()
                ),
                None => println!("{} migración(es) aplicada(s)", report.applied),
            }
        }
        Command::Feed(cmd) => feed(cmd).await?,
        Command::Principal(PrincipalCommand::Create {
            name,
            kind,
            admin,
            data,
        }) => {
            let name = PrincipalName::parse(&name)?;
            let kind = match kind {
                KindArg::User => PrincipalKind::User,
                KindArg::Service => PrincipalKind::Service,
            };
            let store = Store::open(&data.data_dir).await?;
            let p = store
                .create_principal(&name, kind, admin, &local_actor())
                .await?;
            store.close().await;
            println!("principal creado: {} ({})", p.name, p.kind.as_str());
        }
        Command::Token(cmd) => token(cmd).await?,
        Command::Grant(cmd) => grant(cmd).await?,
        Command::Package(cmd) => package(cmd).await?,
        Command::Serve(args) => serve(args).await?,
    }
    Ok(())
}

async fn init(data_dir: &Path) -> CliResult {
    migrate(data_dir).await?;
    restrict_permissions(data_dir, 0o700)?;
    let store = Store::open(data_dir).await?;
    if store.has_admin().await? {
        store.close().await;
        return Err("el directorio de datos ya tiene un administrador; usa `onepackd token create` para emitir otra credencial".into());
    }

    let actor = local_actor();
    let admin = store
        .create_principal(
            &PrincipalName::parse("admin")?,
            PrincipalKind::User,
            true,
            &actor,
        )
        .await?;
    let issued = store
        .create_token(
            &admin,
            Some("credencial inicial"),
            INITIAL_TOKEN_TTL_DAYS * 86_400,
            &actor,
        )
        .await?;
    store.close().await;

    let path = data_dir.join(INITIAL_TOKEN_FILE);
    write_secret_file(&path, &issued.token)?;
    println!(
        "Directorio inicializado. Credencial administrativa inicial (caduca {}) en:\n  {}\n\
         Guárdala en un gestor de secretos y borra el archivo.",
        issued.expires_at,
        path.display()
    );
    Ok(())
}

/// Escribe un secreto en un archivo nuevo, legible solo por el propietario.
fn write_secret_file(path: &Path, secret: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    writeln!(file, "{secret}")?;
    file.sync_all()
}

fn restrict_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

async fn feed(cmd: FeedCommand) -> CliResult {
    match cmd {
        FeedCommand::Create { name, data } => {
            let name = FeedName::parse(&name)?;
            let store = Store::open(&data.data_dir).await?;
            let feed = store.create_feed(&name, &local_actor()).await?;
            store.close().await;
            println!("feed creado: {}", feed.name);
        }
        FeedCommand::List { data } => {
            let store = Store::open(&data.data_dir).await?;
            for feed in store.feeds().await? {
                println!("{}", feed.name);
            }
            store.close().await;
        }
        FeedCommand::Show { name, data } => {
            let store = Store::open(&data.data_dir).await?;
            let feed = require_feed(&store, &name).await?;
            let usage = store.feed_usage(&feed).await?;
            let quota = store.feed_quota(&feed).await?;
            store.close().await;
            let limit = |v: Option<u64>| v.map_or("sin límite".to_owned(), |v| v.to_string());
            println!("feed:            {}", feed.name);
            println!(
                "versiones:       {} (cuota: {})",
                usage.versions,
                limit(quota.max_versions)
            );
            println!(
                "almacenamiento:  {} bytes (cuota: {})",
                usage.storage_bytes,
                limit(quota.max_storage_bytes)
            );
        }
        FeedCommand::Quota {
            name,
            max_storage_mib,
            max_versions,
            data,
        } => {
            let quota = FeedQuota {
                max_storage_bytes: (max_storage_mib > 0).then_some(max_storage_mib << 20),
                max_versions: (max_versions > 0).then_some(max_versions),
            };
            let store = Store::open(&data.data_dir).await?;
            let feed = require_feed(&store, &name).await?;
            store.set_feed_quota(&feed, quota, &local_actor()).await?;
            store.close().await;
            println!("cuotas de {} actualizadas", feed.name);
        }
    }
    Ok(())
}

async fn package(cmd: PackageCommand) -> CliResult {
    let (args, blocked) = match cmd {
        PackageCommand::Block(args) => (args, true),
        PackageCommand::Unblock(args) => (args, false),
    };
    let reason = args.reason.trim();
    if reason.is_empty() {
        return Err("el motivo no puede estar vacío".into());
    }
    let id = PackageId::parse(&args.id)?.identity();
    let version = NuGetVersion::parse(&args.version)?.identity();
    let store = Store::open(&args.data.data_dir).await?;
    let feed = require_feed(&store, &args.feed).await?;
    let change = store
        .set_blocked(&feed, &id, &version, blocked, reason, &local_actor())
        .await?;
    store.close().await;
    let state = if blocked { "bloqueada" } else { "disponible" };
    match change {
        VersionChange::NotFound => {
            Err(format!("no existe {id}@{version} en el feed {}", feed.name).into())
        }
        VersionChange::Unchanged => {
            println!("{id}@{version} ya estaba {state}");
            Ok(())
        }
        VersionChange::Changed => {
            println!("{id}@{version} {state}");
            Ok(())
        }
    }
}

async fn token(cmd: TokenCommand) -> CliResult {
    match cmd {
        TokenCommand::Create {
            principal,
            name,
            expires_in_days,
            data,
        } => {
            let store = Store::open(&data.data_dir).await?;
            let principal = require_principal(&store, &principal).await?;
            let issued = store
                .create_token(
                    &principal,
                    name.as_deref(),
                    expires_in_days * 86_400,
                    &local_actor(),
                )
                .await?;
            store.close().await;
            // Solo el token va a stdout, para poder capturarlo en scripts.
            eprintln!(
                "token {} para {} (caduca {}); no se volverá a mostrar",
                issued.id, principal.name, issued.expires_at
            );
            println!("{}", issued.token);
        }
        TokenCommand::Revoke { id, data } => {
            let store = Store::open(&data.data_dir).await?;
            let revoked = store.revoke_token(&id, &local_actor()).await?;
            store.close().await;
            if !revoked {
                return Err(format!("no existe un token activo con id {id:?}").into());
            }
            println!("token {id} revocado");
        }
    }
    Ok(())
}

async fn grant(cmd: GrantCommand) -> CliResult {
    match cmd {
        GrantCommand::Set {
            principal,
            feed,
            role,
            publish_patterns,
            data,
        } => {
            let role = match role {
                RoleArg::Reader => Role::Reader,
                RoleArg::Publisher => Role::Publisher,
                RoleArg::Maintainer => Role::Maintainer,
            };
            let patterns = publish_patterns
                .iter()
                .map(|p| PublishPattern::parse(p))
                .collect::<Result<Vec<_>, _>>()?;
            let store = Store::open(&data.data_dir).await?;
            let p = require_principal(&store, &principal).await?;
            let f = require_feed(&store, &feed).await?;
            store
                .set_grant(&p, &f, role, &patterns, &local_actor())
                .await?;
            store.close().await;
            println!("{} es {} en {}", p.name, role.as_str(), f.name);
        }
        GrantCommand::Remove {
            principal,
            feed,
            data,
        } => {
            let store = Store::open(&data.data_dir).await?;
            let p = require_principal(&store, &principal).await?;
            let f = require_feed(&store, &feed).await?;
            let removed = store.remove_grant(&p, &f, &local_actor()).await?;
            store.close().await;
            if !removed {
                return Err(format!("{} no tenía acceso a {}", p.name, f.name).into());
            }
            println!("acceso de {} a {} eliminado", p.name, f.name);
        }
    }
    Ok(())
}

async fn serve(args: ServeArgs) -> CliResult {
    let store = Arc::new(Store::open(&args.data.data_dir).await?);
    let free = Store::check_data_dir(&args.data.data_dir, args.min_free_space_mib << 20).await?;
    tracing::info!(free_mib = free >> 20, "directorio de datos verificado");
    let limits = args.limits.limits(args.max_package_size_mib << 20);
    let filled = backfill_metadata(&store, &limits.inspection).await?;
    if filled > 0 {
        tracing::info!(versions = filled, "metadatos rellenados");
    }
    if !store.has_admin().await? {
        tracing::warn!("no hay ningún administrador; ejecuta `onepackd init`");
    }

    tokio::spawn(gc_loop(
        store.clone(),
        Duration::from_secs(args.gc_interval_secs.max(1)),
        Duration::from_secs(args.gc_grace_secs),
    ));

    let router = app(store.clone(), &args.public_url, limits);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, public_url = %args.public_url, "onepackd escuchando");
    // Con la dirección del cliente, para el límite de peticiones por IP.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    store.close().await;
    Ok(())
}
