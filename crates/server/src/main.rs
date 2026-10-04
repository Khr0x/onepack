use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use onepack_core::{FeedName, Principal, PrincipalKind, PrincipalName, PublishPattern, Role};
use onepack_server::{app, gc_loop};
use onepack_storage::{Store, migrate};
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
    }
    Ok(())
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
    if !store.has_admin().await? {
        tracing::warn!("no hay ningún administrador; ejecuta `onepackd init`");
    }

    tokio::spawn(gc_loop(
        store.clone(),
        Duration::from_secs(args.gc_interval_secs.max(1)),
        Duration::from_secs(args.gc_grace_secs),
    ));

    let router = app(
        store.clone(),
        &args.public_url,
        args.max_package_size_mib << 20,
    );
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, public_url = %args.public_url, "onepackd escuchando");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    store.close().await;
    Ok(())
}
