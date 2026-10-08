//! `onepack`: CLI del registro (ADR-015). Datos a stdout, diagnóstico a stderr, `--json`
//! estable y códigos de salida documentados en `docs/cli.md`.

mod app;
mod commands;
mod config;
mod credentials;
mod doctor;
mod error;
mod nuget;
mod nuget_config;
mod output;

use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::app::{App, Global};
use crate::error::exit;
use crate::output::Out;

#[derive(Parser)]
#[command(
    name = "onepack",
    version,
    about = "CLI for the onepack private registry",
    after_help = "Exit codes: 0 ok, 1 error, 2 usage, 3 no credential, 4 no permission, \
                  5 not found, 6 conflict, 7 unavailable, 8 incompatible server, \
                  9 no keychain, 10 doctor found failures."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct GlobalArgs {
    /// Context to use (defaults to the current one).
    #[arg(long, global = true, env = "ONEPACK_CONTEXT")]
    context: Option<String>,
    /// Server URL; takes precedence over the context.
    #[arg(long, global = true, env = "ONEPACK_URL")]
    url: Option<String>,
    /// Environment variable that holds the token (e.g. in pipelines).
    #[arg(long, global = true, value_name = "VAR")]
    token_env: Option<String>,
    /// JSON output (stable schema).
    #[arg(long, global = true)]
    json: bool,
    /// Never prompt: anything that needs confirmation fails without --yes.
    #[arg(
        long,
        global = true,
        env = "ONEPACK_NO_INPUT",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    no_input: bool,
    /// Maximum time per request, in seconds (30 by default; 600 for push).
    #[arg(long, global = true, env = "ONEPACK_TIMEOUT", value_name = "SECS")]
    timeout: Option<u64>,
    /// Extra CA (PEM) for servers with certificates from a private CA.
    #[arg(long, global = true, env = "ONEPACK_CA_CERT", value_name = "PEM")]
    ca_cert: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Configured servers.
    #[command(subcommand)]
    Context(ContextCommand),
    /// Stores the context token in the system keychain.
    Login {
        /// Reads the token from stdin instead of prompting.
        #[arg(long)]
        token_stdin: bool,
    },
    /// Removes the context token from the keychain.
    Logout,
    /// Identity and permissions of the credential in use.
    Whoami,
    /// Feeds and quotas.
    #[command(subcommand)]
    Feed(FeedCommand),
    /// Users and service accounts (admin).
    #[command(subcommand)]
    Principal(PrincipalCommand),
    /// Tokens (admin).
    #[command(subcommand)]
    Token(TokenCommand),
    /// Per-feed permissions (admin).
    #[command(subcommand)]
    Grant(GrantCommand),
    /// Packages and versions.
    #[command(subcommand)]
    Package(PackageCommand),
    /// Audit log (admin).
    #[command(subcommand)]
    Audit(AuditCommand),
    /// NuGet client configuration.
    #[command(subcommand)]
    Nuget(NugetCommand),
    /// Runs a command with the feed's NuGet credentials set only in its environment.
    Exec(ExecArgs),
    /// Diagnoses connection, TLS, credential, permissions and NuGet configuration.
    Doctor(DoctorArgs),
    /// Shell completion script.
    Completion {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
pub enum ContextCommand {
    /// Adds or updates a context.
    Add {
        name: String,
        #[arg(long)]
        url: String,
        /// Extra CA (PEM) for this server.
        #[arg(long, value_name = "PEM")]
        ca_cert: Option<PathBuf>,
        /// Makes it the current context.
        #[arg(long = "use")]
        make_current: bool,
    },
    /// Lists contexts.
    List,
    /// Switches the current context.
    Use { name: String },
    /// Removes a context and its token from the keychain.
    Remove {
        name: String,
        /// Skips the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum FeedCommand {
    /// Creates a feed.
    Create { name: String },
    /// Lists the feeds visible to the credential.
    List,
    /// Shows a feed with its usage and quotas.
    Show { name: String },
    /// Feed quotas. Anything not given is kept; 0 means no limit.
    Configure {
        name: String,
        #[arg(long)]
        max_storage_mib: Option<u64>,
        #[arg(long)]
        max_versions: Option<u64>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum KindArg {
    User,
    Service,
}

#[derive(Subcommand)]
pub enum PrincipalCommand {
    /// Creates a user or a service account.
    Create {
        name: String,
        #[arg(long, value_enum, default_value = "user")]
        kind: KindArg,
        /// Global administrator role.
        #[arg(long)]
        admin: bool,
    },
    /// Lists principals.
    List,
    /// Disables a principal permanently: its tokens stop working.
    Disable {
        name: String,
        /// Skips the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Issues a token. It is shown only once.
    Create {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u32).range(1..=3650))]
        expires_in_days: u32,
    },
    /// Lists active tokens, without secrets.
    List {
        #[arg(long)]
        principal: Option<String>,
    },
    /// Revokes a token (ID: the 16 characters after `opk_`).
    Revoke {
        id: String,
        /// Skips the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum RoleArg {
    Reader,
    Publisher,
    Maintainer,
}

#[derive(Subcommand)]
pub enum GrantCommand {
    /// Assigns (or replaces) a principal's role on a feed.
    Add {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        feed: String,
        #[arg(long, value_enum)]
        role: RoleArg,
        /// Restricts publishing to matching ids (e.g. "Hemia.*"). Repeatable.
        #[arg(long = "publish-pattern")]
        publish_patterns: Vec<String>,
    },
    /// Removes a principal's access to a feed.
    Remove {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        feed: String,
        /// Skips the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Lists grants.
    List {
        #[arg(long)]
        principal: Option<String>,
        #[arg(long)]
        feed: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PackageCommand {
    /// Lists the packages in a feed.
    List {
        #[arg(long)]
        feed: String,
    },
    /// A package's versions, or the details of one version.
    Inspect {
        #[arg(long)]
        feed: String,
        id: String,
        version: Option<String>,
    },
    /// Publishes one or more .nupkg files.
    Push {
        #[arg(long)]
        feed: String,
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// An existing version with the same content is not an error.
        #[arg(long)]
        skip_existing_identical: bool,
    },
    /// Hides a version from search (it can still be downloaded).
    Unlist(VersionArgs),
    /// Shows an unlisted version in search again.
    Relist(VersionArgs),
    /// Prevents a version from being downloaded.
    Block {
        #[command(flatten)]
        version: VersionArgs,
        #[arg(long)]
        reason: String,
        /// Skips the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Allows a blocked version to be downloaded again.
    Unblock {
        #[command(flatten)]
        version: VersionArgs,
        #[arg(long)]
        reason: String,
    },
}

#[derive(Args)]
pub struct VersionArgs {
    #[arg(long)]
    pub feed: String,
    pub id: String,
    pub version: String,
}

#[derive(Subcommand)]
pub enum AuditCommand {
    /// Events, newest first.
    List {
        #[arg(long)]
        feed: Option<String>,
        /// Action prefix (e.g. "package." or "token.create").
        #[arg(long)]
        action: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum NugetCommand {
    /// Adds the feeds to NuGet.Config with packageSourceMapping, without secrets.
    Init {
        /// Feed to add. Repeatable.
        #[arg(long = "feed", required = true)]
        feeds: Vec<String>,
        /// Ids resolved from onepack (e.g. "Hemia.*"). Repeatable.
        #[arg(long = "pattern", required = true)]
        patterns: Vec<String>,
        /// File to modify (defaults to NuGet.Config in the current directory).
        #[arg(long)]
        config: Option<PathBuf>,
        /// Shows the diff without writing.
        #[arg(long)]
        dry_run: bool,
        /// Skips the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct ExecArgs {
    /// Feed whose credentials are injected. Repeatable.
    #[arg(long = "feed", required = true)]
    pub feeds: Vec<String>,
    /// Source key in NuGet.Config (defaults to `onepack_<feed>`). Only with a single feed.
    #[arg(long)]
    pub source_name: Option<String>,
    /// Command and arguments.
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScopeArg {
    Read,
    Publish,
    Maintain,
}

#[derive(Args)]
pub struct DoctorArgs {
    /// Feed to check (permissions, service index, NuGet.Config).
    #[arg(long)]
    pub feed: Option<String>,
    /// Permission the credential must have on the feed.
    #[arg(long, value_enum, default_value = "read")]
    pub require: ScopeArg,
    /// NuGet.Config to check (defaults to the one in the current directory or its parents).
    #[arg(long)]
    pub config: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    let out = Out {
        json: cli.global.json,
        no_input: cli.global.no_input,
    };
    let global = Global {
        context: cli.global.context,
        url: cli.global.url,
        token_env: cli.global.token_env,
        timeout: cli.global.timeout,
        ca_cert: cli.global.ca_cert,
    };
    let code = match run(cli.command, global, out) {
        Ok(code) => code,
        Err(e) => {
            out.error(&e);
            e.exit
        }
    };
    std::process::exit(code);
}

/// Devuelve el código de salida: `exec` propaga el del proceso hijo y `doctor` indica si
/// hubo fallos.
fn run(command: Command, global: Global, out: Out) -> error::CliResult<i32> {
    if let Command::Completion { shell } = command {
        clap_complete::generate(
            shell,
            &mut Cli::command(),
            "onepack",
            &mut std::io::stdout(),
        );
        return Ok(exit::OK);
    }
    let app = App::new(global, out)?;
    match command {
        Command::Context(cmd) => commands::context(app, cmd),
        Command::Login { token_stdin } => commands::login(&app, token_stdin),
        Command::Logout => commands::logout(&app),
        Command::Whoami => commands::whoami(&app),
        Command::Feed(cmd) => commands::feed(&app, cmd),
        Command::Principal(cmd) => commands::principal(&app, cmd),
        Command::Token(cmd) => commands::token(&app, cmd),
        Command::Grant(cmd) => commands::grant(&app, cmd),
        Command::Package(cmd) => commands::package(&app, cmd),
        Command::Audit(cmd) => commands::audit(&app, cmd),
        Command::Nuget(cmd) => nuget::init(&app, cmd),
        Command::Exec(args) => nuget::exec(&app, args),
        Command::Doctor(args) => doctor::run(&app, args),
        Command::Completion { .. } => unreachable!("handled above"),
    }
}
