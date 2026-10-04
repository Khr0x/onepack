use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use onepack_core::FeedName;
use onepack_server::{app, gc_loop};
use onepack_storage::{Store, migrate};
use tracing_subscriber::EnvFilter;

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
    /// Crea o actualiza el esquema del directorio de datos (hace backup si ya había datos).
    Migrate(DataDir),
    /// Gestión local de feeds (la gestión remota llega con la API administrativa).
    #[command(subcommand)]
    Feed(FeedCommand),
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match Cli::parse().command {
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
        Command::Feed(FeedCommand::Create { name, data }) => {
            let name = FeedName::parse(&name)?;
            let store = Store::open(&data.data_dir).await?;
            let feed = store.create_feed(&name).await?;
            store.close().await;
            println!("feed creado: {}", feed.name);
        }
        Command::Feed(FeedCommand::List { data }) => {
            let store = Store::open(&data.data_dir).await?;
            for feed in store.feeds().await? {
                println!("{}", feed.name);
            }
            store.close().await;
        }
        Command::Serve(args) => serve(args).await?,
    }
    Ok(())
}

async fn serve(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(Store::open(&args.data.data_dir).await?);
    let free = Store::check_data_dir(&args.data.data_dir, args.min_free_space_mib << 20).await?;
    tracing::info!(free_mib = free >> 20, "directorio de datos verificado");

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
