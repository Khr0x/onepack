use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use onepack_core::{
    FeedName, FeedQuota, Principal, PrincipalKind, PrincipalName, PublishPattern, Role,
};
use onepack_nuget::{InspectionLimits, NuGetVersion, PackageId};
use onepack_server::{Limits, RateLimit, app_with_ops, backfill_metadata, gc_loop};
use onepack_storage::{
    CheckReport, Store, VersionChange, backup, check, migrate, restore, schema_status,
};
use tracing_subscriber::EnvFilter;

type CliResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Archivo con la credencial administrativa inicial (permisos 0600).
const INITIAL_TOKEN_FILE: &str = "initial-admin-token";
const INITIAL_TOKEN_TTL_DAYS: i64 = 30;

#[derive(Parser)]
#[command(
    name = "onepackd",
    version,
    about = "Server for the onepack private registry"
)]
struct Cli {
    /// Log format (stderr): text for people or JSON for aggregators.
    #[arg(
        long,
        global = true,
        env = "ONEPACK_LOG_FORMAT",
        value_enum,
        default_value = "text"
    )]
    log_format: LogFormat,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Initializes a data directory: migrates the schema and generates the initial
    /// administrative credential.
    Init(DataDir),
    /// Creates or updates the data directory schema (takes a backup if there was data).
    Migrate {
        /// Only reports the schema version and pending migrations.
        #[arg(long)]
        check: bool,
        #[command(flatten)]
        data: DataDir,
    },
    /// Consistent copy of the database and blobs, with a manifest (ADR-017). Enables
    /// maintenance mode while it runs: the server rejects mutations, reads keep working.
    Backup {
        #[command(flatten)]
        data: DataDir,
        /// Destination directory (empty or nonexistent).
        #[arg(long)]
        output: PathBuf,
        /// Maximum maintenance duration if the backup is interrupted.
        #[arg(long, default_value_t = 3600)]
        maintenance_timeout_secs: u64,
    },
    /// Restores a backup into an empty data directory, after verifying the manifest.
    Restore {
        /// Backup directory.
        #[arg(long)]
        from: PathBuf,
        #[command(flatten)]
        data: DataDir,
    },
    /// Integrity between the database and the blobs: missing, corrupt and orphaned.
    Check {
        #[command(flatten)]
        data: DataDir,
        /// Does not read every blob: only existence and size.
        #[arg(long)]
        quick: bool,
        #[arg(long)]
        json: bool,
    },
    /// Manual maintenance mode.
    #[command(subcommand)]
    Maintenance(MaintenanceCommand),
    /// Local feed management.
    #[command(subcommand)]
    Feed(FeedCommand),
    /// Local principal management (users and service accounts).
    #[command(subcommand)]
    Principal(PrincipalCommand),
    /// Local token management.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Local per-feed permission management.
    #[command(subcommand)]
    Grant(GrantCommand),
    /// Local management of published versions.
    #[command(subcommand)]
    Package(PackageCommand),
    /// Starts the server.
    Serve(ServeArgs),
}

#[derive(Args)]
struct DataDir {
    /// Data directory.
    #[arg(long, env = "ONEPACK_DATA_DIR")]
    data_dir: PathBuf,
}

#[derive(Subcommand)]
enum FeedCommand {
    /// Creates a NuGet feed.
    Create {
        name: String,
        #[command(flatten)]
        data: DataDir,
    },
    /// Lists the feeds.
    List {
        #[command(flatten)]
        data: DataDir,
    },
    /// Shows a feed's usage and quotas.
    Show {
        name: String,
        #[command(flatten)]
        data: DataDir,
    },
    /// Sets a feed's quotas (0 = no limit). Does not affect what is already published.
    Quota {
        name: String,
        /// Maximum storage, in MiB (sum of the sizes of all versions).
        #[arg(long)]
        max_storage_mib: u64,
        /// Maximum number of versions.
        #[arg(long)]
        max_versions: u64,
        #[command(flatten)]
        data: DataDir,
    },
}

#[derive(Subcommand)]
enum MaintenanceCommand {
    /// Enables maintenance: the server rejects mutations with 503.
    On {
        #[arg(long)]
        reason: String,
        /// Turns itself off after this time.
        #[arg(long, default_value_t = 3600)]
        duration_secs: u64,
        #[command(flatten)]
        data: DataDir,
    },
    /// Ends maintenance (including one left by an interrupted backup).
    Off {
        #[command(flatten)]
        data: DataDir,
    },
    /// Shows whether maintenance is active.
    Status {
        #[command(flatten)]
        data: DataDir,
    },
}

#[derive(Subcommand)]
enum PackageCommand {
    /// Blocks downloads of a version (it stays visible in the metadata).
    Block(AvailabilityArgs),
    /// Unblocks a version.
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
    /// Reason; recorded in the audit log.
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
    /// Creates a user or a service account.
    Create {
        name: String,
        #[arg(long, value_enum, default_value = "user")]
        kind: KindArg,
        /// Administrator role: manages identities, permissions and every feed.
        #[arg(long)]
        admin: bool,
        #[command(flatten)]
        data: DataDir,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Issues a token. The token is written to stdout and cannot be retrieved later.
    Create {
        #[arg(long)]
        principal: String,
        /// Descriptive name (e.g. "payments pipeline").
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(i64).range(1..=3650))]
        expires_in_days: i64,
        #[command(flatten)]
        data: DataDir,
    },
    /// Revokes a token by its id (the 16 characters after `opk_`).
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
    /// Assigns (or replaces) a principal's role on a feed.
    Set {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        feed: String,
        #[arg(long, value_enum)]
        role: RoleArg,
        /// Restricts publishing to matching ids (e.g. "Hemia.Payments.*"). Repeatable.
        #[arg(long = "publish-pattern")]
        publish_patterns: Vec<String>,
        #[command(flatten)]
        data: DataDir,
    },
    /// Removes a principal's access to a feed.
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
    /// Listen address.
    #[arg(long, env = "ONEPACK_LISTEN", default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// Public URL clients use to reach the server (e.g. https://packages.example.com).
    #[arg(long, env = "ONEPACK_PUBLIC_URL")]
    public_url: String,
    /// Maximum package size, in MiB.
    #[arg(long, env = "ONEPACK_MAX_PACKAGE_SIZE_MIB", default_value_t = 100)]
    max_package_size_mib: u64,
    /// Minimum age, in seconds, of a staging file or an orphaned blob before it is
    /// deleted. It must exceed the duration of any upload: a low value can delete uploads
    /// in progress. Use 0 only in tests.
    #[arg(long, env = "ONEPACK_GC_GRACE_SECS", default_value_t = 3600)]
    gc_grace_secs: u64,
    /// Interval between cleanups, in seconds.
    #[arg(long, env = "ONEPACK_GC_INTERVAL_SECS", default_value_t = 600)]
    gc_interval_secs: u64,
    /// Below this much free space, in MiB, a warning is logged at startup.
    #[arg(long, env = "ONEPACK_MIN_FREE_SPACE_MIB", default_value_t = 1024)]
    min_free_space_mib: u64,
    /// Unauthenticated operations listener: /metrics, /healthz and /readyz. Use it on an
    /// internal interface (e.g. 127.0.0.1:9464).
    #[arg(long, env = "ONEPACK_OPS_LISTEN")]
    ops_listen: Option<SocketAddr>,
    #[command(flatten)]
    limits: LimitArgs,
}

/// Límites frente a paquetes no confiables y saturación (ADR-014).
#[derive(Args)]
struct LimitArgs {
    /// Maximum number of entries in a package ZIP.
    #[arg(long, env = "ONEPACK_MAX_ZIP_ENTRIES", default_value_t = 20_000)]
    max_zip_entries: u64,
    /// Maximum uncompressed size of one package entry, in MiB.
    #[arg(long, env = "ONEPACK_MAX_ENTRY_SIZE_MIB", default_value_t = 512)]
    max_entry_size_mib: u64,
    /// Maximum uncompressed size of the whole package, in MiB.
    #[arg(
        long,
        env = "ONEPACK_MAX_UNCOMPRESSED_SIZE_MIB",
        default_value_t = 2048
    )]
    max_uncompressed_size_mib: u64,
    /// Maximum .nuspec size, in KiB.
    #[arg(long, env = "ONEPACK_MAX_NUSPEC_SIZE_KIB", default_value_t = 1024)]
    max_nuspec_size_kib: u64,
    /// Maximum XML nesting depth of the .nuspec.
    #[arg(long, env = "ONEPACK_MAX_XML_DEPTH", default_value_t = 32)]
    max_xml_depth: usize,
    /// Maximum time to inspect a package, in seconds.
    #[arg(long, env = "ONEPACK_INSPECTION_TIMEOUT_SECS", default_value_t = 30)]
    inspection_timeout_secs: u64,
    /// Concurrent inspections (defaults to half the cores).
    #[arg(long, env = "ONEPACK_MAX_CONCURRENT_INSPECTIONS")]
    max_concurrent_inspections: Option<usize>,
    /// Concurrent uploads; the rest get 503.
    #[arg(long, env = "ONEPACK_MAX_CONCURRENT_UPLOADS", default_value_t = 8)]
    max_concurrent_uploads: usize,
    /// Maximum time to receive an upload, in seconds.
    #[arg(long, env = "ONEPACK_UPLOAD_TIMEOUT_SECS", default_value_t = 600)]
    upload_timeout_secs: u64,
    /// Sustained requests per second per principal (0 = no limit).
    #[arg(long, env = "ONEPACK_PRINCIPAL_RATE_LIMIT", default_value_t = 50)]
    principal_rate_limit: u32,
    /// Maximum burst per principal.
    #[arg(long, env = "ONEPACK_PRINCIPAL_RATE_BURST", default_value_t = 1000)]
    principal_rate_burst: u32,
    /// Sustained requests per second per IP (0 = no limit). Behind a reverse proxy
    /// they all share the proxy's IP.
    #[arg(long, env = "ONEPACK_IP_RATE_LIMIT", default_value_t = 100)]
    ip_rate_limit: u32,
    /// Maximum burst per IP.
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
        .ok_or_else(|| format!("principal {name:?} does not exist"))?)
}

async fn require_feed(store: &Store, name: &str) -> CliResult<onepack_core::Feed> {
    Ok(store
        .feed(name)
        .await?
        .ok_or_else(|| format!("feed {name:?} does not exist"))?)
}

#[tokio::main]
async fn main() -> CliResult {
    let cli = Cli::parse();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    match cli.log_format {
        LogFormat::Text => tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            // Sin códigos de color cuando el log va a un archivo o a journald.
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .with_env_filter(filter)
            .init(),
        // Una línea JSON por evento; los spans incluyen `request_id`.
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_writer(std::io::stderr)
            .with_current_span(false)
            .with_span_list(true)
            .with_env_filter(filter)
            .init(),
    }

    match cli.command {
        Command::Init(DataDir { data_dir }) => init(&data_dir).await?,
        Command::Migrate { check: true, data } => {
            let status = schema_status(&data.data_dir).await?;
            match status.current {
                None => println!(
                    "uninitialized directory; available schema: {}",
                    status.latest
                ),
                Some(v) => println!(
                    "schema {v} (this onepackd version goes up to {})",
                    status.latest
                ),
            }
            if status.pending.is_empty() {
                println!("no pending migrations");
            } else {
                println!(
                    "{} pending migration(s): {:?}; `onepackd migrate` will take a backup first",
                    status.pending.len(),
                    status.pending
                );
            }
        }
        Command::Migrate { check: false, data } => {
            let report = migrate(&data.data_dir).await?;
            match report.backup {
                Some(backup) => println!(
                    "{} migration(s) applied; pre-migration backup in {}",
                    report.applied,
                    backup.display()
                ),
                None => println!("{} migration(s) applied", report.applied),
            }
        }
        Command::Backup {
            data,
            output,
            maintenance_timeout_secs,
        } => {
            let started = std::time::Instant::now();
            let manifest = backup(
                &data.data_dir,
                &output,
                &local_actor(),
                maintenance_timeout_secs,
            )
            .await?;
            let bytes: u64 = manifest.blobs.iter().map(|b| b.size).sum();
            println!(
                "backup in {} ({:.1} s): schema {}, {} feed(s), {} version(s), {} blob(s), {} bytes",
                output.display(),
                started.elapsed().as_secs_f64(),
                manifest.schema_version,
                manifest.feeds,
                manifest.versions,
                manifest.blobs.len(),
                bytes
            );
            println!(
                "It does not include the service configuration (flags or environment variables) or certificates: store them separately."
            );
        }
        Command::Restore { from, data } => {
            let report = restore(&from, &data.data_dir).await?;
            restrict_permissions(&data.data_dir, 0o700)?;
            println!(
                "restored into {}: schema {} ({} migration(s) applied), {} version(s), {} blob(s) verified",
                data.data_dir.display(),
                report.schema_version,
                report.migrations_applied,
                report.versions,
                report.blobs
            );
        }
        Command::Check { data, quick, json } => {
            let report = check(&data.data_dir, !quick).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_check(&report);
            }
            if !report.ok() {
                return Err("the check found missing or corrupt blobs".into());
            }
        }
        Command::Maintenance(cmd) => maintenance(cmd).await?,
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
            println!("principal created: {} ({})", p.name, p.kind.as_str());
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
        return Err("the data directory already has an administrator; use `onepackd token create` to issue another credential".into());
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
            Some("initial credential"),
            INITIAL_TOKEN_TTL_DAYS * 86_400,
            &actor,
        )
        .await?;
    store.close().await;

    let path = data_dir.join(INITIAL_TOKEN_FILE);
    write_secret_file(&path, &issued.token)?;
    println!(
        "Directory initialized. Initial administrative credential (expires {}) in:\n  {}\n\
         Store it in a secrets manager and delete the file.",
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
            println!("feed created: {}", feed.name);
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
            let limit = |v: Option<u64>| v.map_or("no limit".to_owned(), |v| v.to_string());
            println!("feed:            {}", feed.name);
            println!(
                "versions:        {} (quota: {})",
                usage.versions,
                limit(quota.max_versions)
            );
            println!(
                "storage:         {} bytes (quota: {})",
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
            println!("quotas for {} updated", feed.name);
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
        return Err("the reason cannot be empty".into());
    }
    let id = PackageId::parse(&args.id)?.identity();
    let version = NuGetVersion::parse(&args.version)?.identity();
    let store = Store::open(&args.data.data_dir).await?;
    let feed = require_feed(&store, &args.feed).await?;
    let change = store
        .set_blocked(&feed, &id, &version, blocked, reason, &local_actor())
        .await?;
    store.close().await;
    let state = if blocked { "blocked" } else { "available" };
    match change {
        VersionChange::NotFound => {
            Err(format!("{id}@{version} does not exist in feed {}", feed.name).into())
        }
        VersionChange::Unchanged => {
            println!("{id}@{version} was already {state}");
            Ok(())
        }
        VersionChange::Changed => {
            println!("{id}@{version} {state}");
            Ok(())
        }
    }
}

fn print_check(report: &CheckReport) {
    println!(
        "{} version(s), {} blob(s) checked{}",
        report.versions,
        report.blobs_checked,
        if report.hashes_verified {
            " with hash verification"
        } else {
            " (hashes not verified: --quick)"
        }
    );
    for (label, problems) in [("MISSING", &report.missing), ("CORRUPT", &report.corrupt)] {
        for p in problems {
            println!(
                "{label} {} ({}): {}",
                p.sha256,
                p.detail,
                p.versions.join(", ")
            );
        }
    }
    if !report.orphan_files.is_empty() {
        println!(
            "{} orphaned blob(s) (cleanup deletes them after the grace period)",
            report.orphan_files.len()
        );
    }
    if report.staging_files > 0 {
        println!("{} file(s) in staging", report.staging_files);
    }
    println!(
        "{}",
        if report.ok() {
            "Integrity OK."
        } else {
            "Some versions cannot be downloaded: restore their blobs from a backup."
        }
    );
}

async fn maintenance(cmd: MaintenanceCommand) -> CliResult {
    match cmd {
        MaintenanceCommand::On {
            reason,
            duration_secs,
            data,
        } => {
            let store = Store::open(&data.data_dir).await?;
            let result = store
                .begin_maintenance(&reason, duration_secs, &local_actor())
                .await?;
            store.close().await;
            match result {
                Ok(()) => println!("maintenance active ({reason}) for at most {duration_secs} s"),
                Err(active) => {
                    return Err(format!(
                        "maintenance is already in progress: {} (until {})",
                        active.reason, active.expires_at
                    )
                    .into());
                }
            }
        }
        MaintenanceCommand::Off { data } => {
            let store = Store::open(&data.data_dir).await?;
            let ended = store.end_maintenance(&local_actor()).await?;
            store.close().await;
            println!(
                "{}",
                if ended {
                    "maintenance ended"
                } else {
                    "there was no maintenance"
                }
            );
        }
        MaintenanceCommand::Status { data } => {
            let store = Store::open(&data.data_dir).await?;
            let state = store.maintenance().await?;
            store.close().await;
            match state {
                Some(m) => println!(
                    "maintenance active: {} (by {}, since {}, expires {})",
                    m.reason,
                    m.actor.as_deref().unwrap_or("?"),
                    m.started_at,
                    m.expires_at
                ),
                None => println!("no maintenance"),
            }
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
                "token {} for {} (expires {}); it will not be shown again",
                issued.id, principal.name, issued.expires_at
            );
            println!("{}", issued.token);
        }
        TokenCommand::Revoke { id, data } => {
            let store = Store::open(&data.data_dir).await?;
            let revoked = store.revoke_token(&id, &local_actor()).await?;
            store.close().await;
            if !revoked {
                return Err(format!("there is no active token with id {id:?}").into());
            }
            println!("token {id} revoked");
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
            println!("{} is {} on {}", p.name, role.as_str(), f.name);
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
                return Err(format!("{} had no access to {}", p.name, f.name).into());
            }
            println!("{}'s access to {} removed", p.name, f.name);
        }
    }
    Ok(())
}

async fn serve(args: ServeArgs) -> CliResult {
    let store = Arc::new(Store::open(&args.data.data_dir).await?);
    let free = Store::check_data_dir(&args.data.data_dir, args.min_free_space_mib << 20).await?;
    tracing::info!(free_mib = free >> 20, "data directory verified");
    let limits = args.limits.limits(args.max_package_size_mib << 20);
    let filled = backfill_metadata(&store, &limits.inspection).await?;
    if filled > 0 {
        tracing::info!(versions = filled, "metadata backfilled");
    }
    if !store.has_admin().await? {
        tracing::warn!("there is no administrator; run `onepackd init`");
    }

    tokio::spawn(gc_loop(
        store.clone(),
        Duration::from_secs(args.gc_interval_secs.max(1)),
        Duration::from_secs(args.gc_grace_secs),
    ));

    let (router, ops_router) = app_with_ops(store.clone(), &args.public_url, limits);
    if let Some(addr) = args.ops_listen {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(listen = %addr, "operations listener (/metrics, /healthz, /readyz)");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, ops_router).await {
                tracing::error!(error = %e, "the operations listener stopped");
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, public_url = %args.public_url, "onepackd listening");
    // Con la dirección del cliente, para el límite de peticiones por IP.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    store.close().await;
    tracing::info!("onepackd stopped");
    Ok(())
}

/// Ctrl+C o SIGTERM (lo que envía systemd al parar el servicio).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("stopping: finishing requests in progress");
}
