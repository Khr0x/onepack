use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use onepack_server::{Config, app};
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
    /// Inicia el servidor.
    Serve(ServeArgs),
}

#[derive(Args)]
struct ServeArgs {
    /// Dirección de escucha.
    #[arg(long, env = "ONEPACK_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// Directorio de datos.
    #[arg(long, env = "ONEPACK_DATA_DIR")]
    data_dir: PathBuf,
    /// URL pública con la que los clientes acceden al servidor (p. ej. https://packages.example.com).
    #[arg(long, env = "ONEPACK_PUBLIC_URL")]
    public_url: String,
    /// Nombre del feed (en la Fase 1 hay un único feed).
    #[arg(long, env = "ONEPACK_FEED", default_value = "default")]
    feed: String,
    /// Tamaño máximo de un paquete, en MiB.
    #[arg(long, env = "ONEPACK_MAX_PACKAGE_SIZE_MIB", default_value_t = 100)]
    max_package_size_mib: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let Command::Serve(args) = Cli::parse().command;
    let router = app(Config {
        data_dir: &args.data_dir,
        public_url: &args.public_url,
        feed: &args.feed,
        max_package_bytes: args.max_package_size_mib * 1024 * 1024,
    })
    .await?;

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, public_url = %args.public_url, feed = %args.feed, "onepackd escuchando");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
