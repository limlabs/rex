#[cfg(feature = "build")]
mod cmd_build;
#[cfg(feature = "dev")]
mod cmd_dev;
#[cfg(feature = "lint")]
mod cmd_fmt;
mod cmd_init;
#[cfg(feature = "lint")]
mod cmd_lint;
#[cfg(feature = "live")]
mod cmd_live;
mod cmd_start;
mod display;
#[cfg(feature = "build")]
mod export;
#[cfg(feature = "dev")]
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};
#[cfg(feature = "build")]
use rex_core::ProjectConfig;
use std::net::IpAddr;
use std::path::PathBuf;
#[cfg(feature = "dev")]
use tracing_subscriber::layer::SubscriberExt;
#[cfg(feature = "dev")]
use tracing_subscriber::util::SubscriberInitExt;
#[cfg(feature = "dev")]
use tui::log_layer::{LogBuffer, TuiLogLayer};

#[derive(Parser)]
#[command(name = "rex", about = "Rex - Next.js Pages Router in Rust")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the development server with HMR
    #[cfg(feature = "dev")]
    Dev {
        /// Port to listen on (also reads $PORT env var)
        #[arg(short, long, default_value = "3000", env = "PORT")]
        port: u16,

        /// Host to bind to (also reads $HOST env var)
        #[arg(short = 'H', long, default_value = "127.0.0.1", env = "HOST")]
        host: IpAddr,

        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Disable TUI (use plain log output)
        #[arg(long)]
        no_tui: bool,
    },

    /// Create a production build
    #[cfg(feature = "build")]
    Build {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },

    /// Export a static site (pre-render all pages to HTML)
    #[cfg(feature = "build")]
    Export {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Output directory (default: .rex/export)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Continue even if some pages can't be statically exported
        #[arg(long)]
        force: bool,

        /// Base path prefix for asset URLs (e.g. "/rex" for GitHub Pages)
        #[arg(long, default_value = "")]
        base_path: String,

        /// Append .html extensions to internal navigation links.
        /// Most static hosts (GitHub Pages, Netlify, Vercel) handle clean URLs
        /// automatically, so this is off by default. Enable for hosts that
        /// require explicit .html extensions (e.g. S3, basic nginx).
        #[arg(long)]
        html_extensions: bool,
    },

    /// Start the production server
    Start {
        /// Port to listen on (also reads $PORT env var)
        #[arg(short, long, default_value = "3000", env = "PORT")]
        port: u16,

        /// Host to bind to
        #[arg(short = 'H', long, default_value = "0.0.0.0", env = "HOST")]
        host: IpAddr,

        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },

    /// Lint source files with oxlint (React + Next.js rules)
    #[cfg(feature = "lint")]
    Lint {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Fix auto-fixable problems
        #[arg(long)]
        fix: bool,

        /// Treat warnings as errors
        #[arg(long)]
        deny_warnings: bool,

        /// Paths to lint (defaults to pages/ directory)
        #[arg(trailing_var_arg = true)]
        paths: Vec<PathBuf>,
    },

    /// Type-check pages with tsc
    #[cfg(feature = "dev")]
    Typecheck {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Additional arguments passed to tsc
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Serve React apps from source with on-demand compilation
    #[cfg(feature = "live")]
    Live {
        /// Mount a project at a path prefix: PREFIX=SOURCE (repeatable)
        #[arg(short = 'm', long = "mount", value_name = "PREFIX=SOURCE")]
        mount: Vec<String>,

        /// Port to listen on
        #[arg(short, long, default_value = "3000", env = "PORT")]
        port: u16,

        /// Host to bind to
        #[arg(short = 'H', long, default_value = "0.0.0.0", env = "HOST")]
        host: IpAddr,

        /// V8 workers per project
        #[arg(long, default_value = "2")]
        workers: usize,
    },

    /// Create a new Rex project
    Init {
        /// Project name (creates a new directory)
        name: String,
    },

    /// Format source files with oxfmt
    #[cfg(feature = "lint")]
    Fmt {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Check formatting without writing (exits with error if unformatted)
        #[arg(long)]
        check: bool,

        /// Format a single file instead of discovering files
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

const DEFAULT_LOG_FILTER: &str = "rex=info,v8::console=info";

fn init_plain_tracing() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| DEFAULT_LOG_FILTER.into()),
        )
        .init();
}

#[cfg(feature = "dev")]
fn init_tui_tracing() -> LogBuffer {
    let buffer = LogBuffer::new(1000);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| DEFAULT_LOG_FILTER.into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(TuiLogLayer::new(buffer.clone()))
        .init();

    buffer
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        #[cfg(feature = "dev")]
        Commands::Dev {
            port,
            host,
            root,
            no_tui,
        } => {
            let root = std::fs::canonicalize(&root)?;
            load_dotenv(&root);
            let project_config = ProjectConfig::load(&root)?;
            let is_terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
            let tui_enabled = !no_tui && !project_config.dev.no_tui && is_terminal;

            if tui_enabled {
                let log_buffer = init_tui_tracing();
                let log_buffer_fallback = log_buffer.clone();
                let result = cmd_dev::cmd_dev(root, port, host, true, Some(log_buffer)).await;
                if result.is_err() {
                    // The TUI never started — dump buffered logs to stderr
                    // so the user can see what happened before the error.
                    for entry in log_buffer_fallback.snapshot() {
                        eprintln!("[{}] {}", entry.level, entry.message);
                    }
                }
                result
            } else {
                init_plain_tracing();
                cmd_dev::cmd_dev(root, port, host, false, None).await
            }
        }
        #[cfg(feature = "build")]
        Commands::Build { root } => {
            let root_abs = std::fs::canonicalize(&root).unwrap_or(root.clone());
            load_dotenv(&root_abs);
            init_plain_tracing();
            cmd_build::cmd_build(root).await
        }
        #[cfg(feature = "build")]
        Commands::Export {
            root,
            output,
            force,
            base_path,
            html_extensions,
        } => {
            init_plain_tracing();
            export::cmd_export(root, output, force, base_path, html_extensions).await
        }
        Commands::Start { port, host, root } => {
            let root_abs = std::fs::canonicalize(&root).unwrap_or(root.clone());
            load_dotenv(&root_abs);
            init_plain_tracing();
            cmd_start::cmd_start(root, port, host).await
        }
        #[cfg(feature = "lint")]
        Commands::Lint {
            root,
            fix,
            deny_warnings,
            paths,
        } => {
            init_plain_tracing();
            cmd_lint::cmd_lint(root, fix, deny_warnings, paths)
        }
        #[cfg(feature = "dev")]
        Commands::Typecheck { root, args } => {
            init_plain_tracing();
            cmd_dev::cmd_typecheck(root, args)
        }
        #[cfg(feature = "live")]
        Commands::Live {
            mount,
            port,
            host,
            workers,
        } => {
            init_plain_tracing();
            cmd_live::cmd_live(mount, port, host, workers).await
        }
        Commands::Init { name } => {
            init_plain_tracing();
            cmd_init::cmd_init(name)
        }
        #[cfg(feature = "lint")]
        Commands::Fmt { root, check, file } => {
            init_plain_tracing();
            if let Some(file) = file {
                cmd_fmt::cmd_fmt_file(file, root, check)
            } else {
                cmd_fmt::cmd_fmt(root, check)
            }
        }
    }
}

/// Load `.env` file from the project root into the process environment.
/// Follows Next.js behavior: variables already set in the environment
/// are NOT overwritten (env vars take precedence over `.env` file).
fn load_dotenv(project_root: &std::path::Path) {
    let env_path = project_root.join(".env");
    let contents = match std::fs::read_to_string(&env_path) {
        Ok(c) => c,
        Err(_) => return, // No .env file — that's fine
    };

    for line in contents.lines() {
        let line = line.trim();
        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split on first '='
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = value.trim();
        // Strip surrounding quotes (must be at least 2 chars to have open+close)
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        // Don't overwrite existing env vars
        if std::env::var(key).is_err() {
            std::env::set_var(key, value);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_dotenv_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(tmp.path().join(".env")).unwrap();
        writeln!(f, "REX_TEST_BASIC=hello").unwrap();
        writeln!(f, "REX_TEST_QUOTED=\"world\"").unwrap();
        writeln!(f, "REX_TEST_SINGLE='single'").unwrap();
        writeln!(f, "# comment line").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "REX_TEST_SPACES = spaced ").unwrap();
        drop(f);

        // Clear any pre-existing values
        std::env::remove_var("REX_TEST_BASIC");
        std::env::remove_var("REX_TEST_QUOTED");
        std::env::remove_var("REX_TEST_SINGLE");
        std::env::remove_var("REX_TEST_SPACES");

        load_dotenv(tmp.path());

        assert_eq!(std::env::var("REX_TEST_BASIC").unwrap(), "hello");
        assert_eq!(std::env::var("REX_TEST_QUOTED").unwrap(), "world");
        assert_eq!(std::env::var("REX_TEST_SINGLE").unwrap(), "single");
        assert_eq!(std::env::var("REX_TEST_SPACES").unwrap(), "spaced");

        // Cleanup
        std::env::remove_var("REX_TEST_BASIC");
        std::env::remove_var("REX_TEST_QUOTED");
        std::env::remove_var("REX_TEST_SINGLE");
        std::env::remove_var("REX_TEST_SPACES");
    }

    #[test]
    fn load_dotenv_single_char_quote_no_panic() {
        let tmp = tempfile::tempdir().unwrap();
        // A value that is just a single quote character — previously caused a panic
        std::fs::write(tmp.path().join(".env"), "REX_TEST_QUOTE=\"\n").unwrap();

        std::env::remove_var("REX_TEST_QUOTE");
        // Must not panic; single `"` is not a matching pair so it stays as-is
        load_dotenv(tmp.path());
        assert_eq!(std::env::var("REX_TEST_QUOTE").unwrap(), "\"");
        std::env::remove_var("REX_TEST_QUOTE");
    }

    #[test]
    fn load_dotenv_does_not_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), "REX_TEST_NOOVER=file_value\n").unwrap();

        std::env::set_var("REX_TEST_NOOVER", "env_value");
        load_dotenv(tmp.path());
        assert_eq!(std::env::var("REX_TEST_NOOVER").unwrap(), "env_value");
        std::env::remove_var("REX_TEST_NOOVER");
    }

    #[test]
    fn load_dotenv_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        // Should not panic or error when .env doesn't exist
        load_dotenv(tmp.path());
    }

    #[test]
    fn load_dotenv_empty_key_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        // "=value" produces an empty key — must not panic
        std::fs::write(tmp.path().join(".env"), "=bad_value\nREX_TEST_EKEY=good\n").unwrap();

        std::env::remove_var("REX_TEST_EKEY");
        load_dotenv(tmp.path());
        assert_eq!(std::env::var("REX_TEST_EKEY").unwrap(), "good");
        std::env::remove_var("REX_TEST_EKEY");
    }
}
