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
    about = "CLI del registro privado onepack",
    after_help = "Códigos de salida: 0 ok, 1 error, 2 uso, 3 sin credencial, 4 sin permiso, \
                  5 no encontrado, 6 conflicto, 7 no disponible, 8 servidor incompatible, \
                  9 sin keychain, 10 doctor con fallos."
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct GlobalArgs {
    /// Contexto a usar (por defecto, el actual).
    #[arg(long, global = true, env = "ONEPACK_CONTEXT")]
    context: Option<String>,
    /// URL del servidor; tiene prioridad sobre el contexto.
    #[arg(long, global = true, env = "ONEPACK_URL")]
    url: Option<String>,
    /// Variable de entorno que contiene el token (p. ej. en pipelines).
    #[arg(long, global = true, value_name = "VAR")]
    token_env: Option<String>,
    /// Salida en JSON (esquema estable).
    #[arg(long, global = true)]
    json: bool,
    /// No pedir nada por terminal: lo que requiera confirmación falla sin --yes.
    #[arg(
        long,
        global = true,
        env = "ONEPACK_NO_INPUT",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    no_input: bool,
    /// Tiempo máximo por petición, en segundos (30 por defecto; 600 en push).
    #[arg(long, global = true, env = "ONEPACK_TIMEOUT", value_name = "SECS")]
    timeout: Option<u64>,
    /// CA adicional (PEM) para servidores con certificados de una CA propia.
    #[arg(long, global = true, env = "ONEPACK_CA_CERT", value_name = "PEM")]
    ca_cert: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Servidores configurados.
    #[command(subcommand)]
    Context(ContextCommand),
    /// Guarda el token del contexto en el keychain del sistema.
    Login {
        /// Lee el token de stdin en lugar de pedirlo por terminal.
        #[arg(long)]
        token_stdin: bool,
    },
    /// Borra el token del contexto del keychain.
    Logout,
    /// Identidad y permisos de la credencial en uso.
    Whoami,
    /// Feeds.
    #[command(subcommand)]
    Feed(FeedCommand),
    /// Usuarios y cuentas de servicio (administración).
    #[command(subcommand)]
    Principal(PrincipalCommand),
    /// Tokens (administración).
    #[command(subcommand)]
    Token(TokenCommand),
    /// Permisos por feed (administración).
    #[command(subcommand)]
    Grant(GrantCommand),
    /// Paquetes y versiones.
    #[command(subcommand)]
    Package(PackageCommand),
    /// Registro de auditoría (administración).
    #[command(subcommand)]
    Audit(AuditCommand),
    /// Configuración de clientes NuGet.
    #[command(subcommand)]
    Nuget(NugetCommand),
    /// Ejecuta un comando con las credenciales NuGet del feed solo en su entorno.
    Exec(ExecArgs),
    /// Diagnostica conexión, TLS, credencial, permisos y configuración NuGet.
    Doctor(DoctorArgs),
    /// Script de autocompletado para la shell.
    Completion {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Subcommand)]
pub enum ContextCommand {
    /// Añade o actualiza un contexto.
    Add {
        name: String,
        #[arg(long)]
        url: String,
        /// CA adicional (PEM) para este servidor.
        #[arg(long, value_name = "PEM")]
        ca_cert: Option<PathBuf>,
        /// Lo convierte en el contexto actual.
        #[arg(long = "use")]
        make_current: bool,
    },
    List,
    /// Cambia el contexto actual.
    Use {
        name: String,
    },
    /// Elimina un contexto y su token del keychain.
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum FeedCommand {
    Create {
        name: String,
    },
    List,
    Show {
        name: String,
    },
    /// Cuotas del feed. Lo que no se indique se mantiene; 0 es sin límite.
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
    Create {
        name: String,
        #[arg(long, value_enum, default_value = "user")]
        kind: KindArg,
        /// Rol Administrator.
        #[arg(long)]
        admin: bool,
    },
    List,
    /// Desactiva un principal: sus tokens dejan de funcionar.
    Disable {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Emite un token. Se muestra una sola vez.
    Create {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u32).range(1..=3650))]
        expires_in_days: u32,
    },
    List {
        #[arg(long)]
        principal: Option<String>,
    },
    Revoke {
        id: String,
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
    /// Asigna (o reemplaza) el rol de un principal en un feed.
    Add {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        feed: String,
        #[arg(long, value_enum)]
        role: RoleArg,
        /// Restringe la publicación a ids que coincidan (p. ej. "Hemia.*"). Repetible.
        #[arg(long = "publish-pattern")]
        publish_patterns: Vec<String>,
    },
    Remove {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        feed: String,
        #[arg(long)]
        yes: bool,
    },
    List {
        #[arg(long)]
        principal: Option<String>,
        #[arg(long)]
        feed: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PackageCommand {
    List {
        #[arg(long)]
        feed: String,
    },
    /// Versiones de un paquete, o el detalle de una versión.
    Inspect {
        #[arg(long)]
        feed: String,
        id: String,
        version: Option<String>,
    },
    /// Publica uno o varios .nupkg.
    Push {
        #[arg(long)]
        feed: String,
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Si la versión ya existe con el mismo contenido, no es un error.
        #[arg(long)]
        skip_existing_identical: bool,
    },
    /// Oculta una versión de la búsqueda (sigue descargable).
    Unlist(VersionArgs),
    Relist(VersionArgs),
    /// Impide descargar una versión.
    Block {
        #[command(flatten)]
        version: VersionArgs,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        yes: bool,
    },
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
    /// Eventos del más reciente al más antiguo.
    List {
        #[arg(long)]
        feed: Option<String>,
        /// Prefijo de la acción (p. ej. "package." o "token.create").
        #[arg(long)]
        action: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

#[derive(Subcommand)]
pub enum NugetCommand {
    /// Añade los feeds a NuGet.Config con packageSourceMapping, sin secretos.
    Init {
        /// Feed a añadir. Repetible.
        #[arg(long = "feed", required = true)]
        feeds: Vec<String>,
        /// Ids que se resuelven desde onepack (p. ej. "Hemia.*"). Repetible.
        #[arg(long = "pattern", required = true)]
        patterns: Vec<String>,
        /// Archivo a modificar (por defecto, el NuGet.Config del directorio actual).
        #[arg(long)]
        config: Option<PathBuf>,
        /// Muestra el diff sin escribir.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct ExecArgs {
    /// Feed cuyas credenciales se inyectan. Repetible.
    #[arg(long = "feed", required = true)]
    pub feeds: Vec<String>,
    /// Clave de la fuente en NuGet.Config (por defecto `onepack_<feed>`). Solo con un feed.
    #[arg(long)]
    pub source_name: Option<String>,
    /// Comando y argumentos.
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
    /// Feed a comprobar (permisos, service index, NuGet.Config).
    #[arg(long)]
    pub feed: Option<String>,
    /// Permiso que debe tener la credencial en el feed.
    #[arg(long, value_enum, default_value = "read")]
    pub require: ScopeArg,
    /// NuGet.Config a revisar (por defecto, el del directorio actual o superiores).
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
        Command::Completion { .. } => unreachable!("atendido arriba"),
    }
}
