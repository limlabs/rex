#[cfg(feature = "build")]
mod export;
#[cfg(feature = "dev")]
mod tui;

#[cfg(feature = "dev")]
use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
#[cfg(feature = "lint")]
use rayon::prelude::*;
#[cfg(feature = "build")]
use rex_build::build_bundles;
#[cfg(feature = "build")]
use rex_core::{AssetManifest, ProjectConfig, RexConfig};
#[cfg(feature = "build")]
use rex_router::scan_project;
use std::net::IpAddr;
use std::path::PathBuf;
#[cfg(feature = "dev")]
use std::process::Command;
#[cfg(feature = "lint")]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(feature = "build")]
use tracing::debug;
#[cfg(feature = "dev")]
use tracing::info;
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
                let result = cmd_dev(root, port, host, true, Some(log_buffer)).await;
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
                cmd_dev(root, port, host, false, None).await
            }
        }
        #[cfg(feature = "build")]
        Commands::Build { root } => {
            let root_abs = std::fs::canonicalize(&root).unwrap_or(root.clone());
            load_dotenv(&root_abs);
            init_plain_tracing();
            cmd_build(root).await
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
            cmd_start(root, port, host).await
        }
        #[cfg(feature = "lint")]
        Commands::Lint {
            root,
            fix,
            deny_warnings,
            paths,
        } => {
            init_plain_tracing();
            cmd_lint(root, fix, deny_warnings, paths)
        }
        #[cfg(feature = "dev")]
        Commands::Typecheck { root, args } => {
            init_plain_tracing();
            cmd_typecheck(root, args)
        }
        #[cfg(feature = "live")]
        Commands::Live {
            mount,
            port,
            host,
            workers,
        } => {
            init_plain_tracing();
            cmd_live(mount, port, host, workers).await
        }
        Commands::Init { name } => {
            init_plain_tracing();
            cmd_init(name)
        }
        #[cfg(feature = "lint")]
        Commands::Fmt { root, check } => {
            init_plain_tracing();
            cmd_fmt(root, check)
        }
    }
}

#[cfg(feature = "dev")]
async fn cmd_dev(
    root: PathBuf,
    port: u16,
    host: IpAddr,
    tui_enabled: bool,
    log_buffer: Option<LogBuffer>,
) -> Result<()> {
    let start = std::time::Instant::now();

    // Bind the port early — fail fast on conflicts before the expensive build.
    let addr = std::net::SocketAddr::new(host, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr} — is another server running?"))?;

    if !tui_enabled {
        print_mascot_header(env!("CARGO_PKG_VERSION"), "");
    }

    let rex = rex_server::Rex::new(rex_server::RexOptions {
        root: root.clone(),
        dev: true,
        port,
        host,
    })
    .await?;

    let config = rex.config().clone();
    let scan = rex.scan().clone();
    let state = rex.state();

    // Start Tailwind CSS watch process (if Tailwind is detected)
    let _tailwind = rex_dev::TailwindProcess::start(&config, &scan)?;
    if !tui_enabled && _tailwind.is_some() {
        eprintln!("  {} {}", dim("◇"), dim("Tailwind CSS (watching)"));
    }

    // Create HMR broadcast
    let hmr = rex_dev::HmrBroadcast::new();

    // Start tsc watch process (if TypeScript is detected)
    let _tsc = rex_dev::typecheck::spawn_tsc_watcher(&config.project_root, hmr.clone());
    if !tui_enabled && _tsc.is_some() {
        eprintln!("  {} {}", dim("◇"), dim("TypeScript (watching)"));
    }

    // Start file watcher (watches project root for CSS changes too)
    let event_rx =
        rex_dev::start_watcher(&config.project_root, &config.pages_dir, &config.app_dir)?;

    // Bridge sync file watcher to async rebuild handler
    let (rebuild_tx, mut rebuild_rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::task::spawn_blocking(move || {
        while let Ok(event) = event_rx.recv() {
            let _ = rebuild_tx.send(event);
        }
    });

    // Build server with HMR websocket route
    let hmr_route = axum::routing::get({
        let hmr_clone = hmr.clone();
        move |ws: axum::extract::ws::WebSocketUpgrade| async move {
            ws.on_upgrade(move |socket| rex_dev::hmr::handle_hmr_socket(socket, hmr_clone))
        }
    });

    let extra_routes = axum::Router::new().route("/_rex/hmr", hmr_route).route(
        "/_rex/hmr-client.js",
        axum::routing::get(hmr_client_handler),
    );

    let router = rex.router_with_extra(extra_routes);

    // Spawn async rebuild handler: rebuild bundles + reload V8 isolates on file changes
    {
        let rebuild_config = config.clone();
        let rebuild_hmr = hmr.clone();
        let rebuild_state = state.clone();
        tokio::spawn(async move {
            while let Some(event) = rebuild_rx.recv().await {
                info!(path = %event.path.display(), "Rebuilding...");
                match rex_dev::rebuild::handle_file_event(
                    event,
                    &rebuild_config,
                    &rebuild_state,
                    &rebuild_hmr,
                )
                .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::error!("Rebuild failed: {e}");
                        rebuild_hmr.send_error(&e.to_string(), None);
                    }
                }
            }
        });
    }

    let elapsed = start.elapsed();

    if tui_enabled {
        // Spawn the HTTP server as a background task
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                tracing::error!("Server error: {e}");
            }
        });

        let startup_info = tui::StartupInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            ready_ms: elapsed.as_millis() as u64,
            url: format!("http://localhost:{port}"),
            page_count: scan.routes.len(),
            api_count: scan.api_routes.len(),
            has_tailwind: _tailwind.is_some(),
            has_typescript: _tsc.is_some(),
        };

        if let Some(buf) = log_buffer {
            tui::run_tui(buf, startup_info).await?;
        }

        // The TUI has exited — force-quit the process. Background tasks
        // (file watcher, rebuild handler, HTTP server) have no shutdown
        // signal and would block the tokio runtime's graceful shutdown.
        std::process::exit(0);
    } else {
        eprintln!(
            "  {} {}",
            green_bold("✓ Ready in"),
            green_bold(&format_duration(elapsed))
        );
        eprintln!();
        eprintln!(
            "  {} {}",
            dim("➜ Local:"),
            bold(&format!("http://localhost:{port}"))
        );
        eprintln!();
        print_route_summary(&scan.routes, &scan.api_routes);
        eprintln!();

        axum::serve(listener, router).await?;
    }

    Ok(())
}

#[cfg(feature = "build")]
async fn cmd_build(root: PathBuf) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root);
    config.validate()?;

    print_mascot_header(env!("CARGO_PKG_VERSION"), "");

    let start = std::time::Instant::now();
    debug!("Building for production...");

    let scan = scan_project(&config.project_root, &config.pages_dir, &config.app_dir)?;
    debug!(routes = scan.routes.len(), "Routes scanned");

    let project_config = ProjectConfig::load(&config.project_root)?;
    let build_result = build_bundles(&config, &scan, &project_config).await?;
    let elapsed = start.elapsed();

    eprintln!(
        "  {} {}",
        green_bold("✓ Built in"),
        green_bold(&format_duration(elapsed))
    );
    eprintln!();

    // Server bundle size
    let server_size = std::fs::metadata(&build_result.server_bundle_path)
        .map(|m| m.len())
        .unwrap_or(0);
    eprintln!("  {}  {}", dim("Server"), format_size(server_size));

    // Client bundle sizes
    let client_dir = config.client_build_dir();
    let mut total_client: u64 = 0;
    let mut page_sizes: Vec<(String, u64)> = Vec::new();
    let mut chunk_sizes: Vec<(String, u64)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&client_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "js") {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                total_client += size;
                if name.starts_with("chunk-") {
                    chunk_sizes.push((name, size));
                } else {
                    page_sizes.push((name, size));
                }
            }
        }
    }

    eprintln!("  {}  {}", dim("Client"), format_size(total_client));
    eprintln!();

    // Show page entry chunks
    page_sizes.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, size) in &page_sizes {
        eprintln!(
            "    {}  {}",
            dim(&format!("{:<38}", name)),
            dim(&format_size(*size))
        );
    }

    // Show shared chunks
    chunk_sizes.sort_by(|a, b| b.1.cmp(&a.1)); // largest first
    for (name, size) in &chunk_sizes {
        eprintln!(
            "    {}  {}",
            dim(&format!("{:<38}", name)),
            dim(&format_size(*size))
        );
    }

    eprintln!();
    print_route_summary_with_manifest(&scan.routes, &scan.api_routes, &build_result.manifest);
    eprintln!();
    Ok(())
}

fn cmd_init(name: String) -> Result<()> {
    let project_dir = PathBuf::from(&name);

    if project_dir.exists() {
        anyhow::bail!("Directory '{}' already exists", name);
    }

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex"), dim("creating project..."));
    eprintln!();

    // Create directory structure — no package.json needed.
    // Rex embeds React and extracts it automatically on first run.
    std::fs::create_dir_all(project_dir.join("pages"))?;
    std::fs::create_dir_all(project_dir.join("public"))?;

    // .gitignore
    std::fs::write(
        project_dir.join(".gitignore"),
        "node_modules\n.rex\n.DS_Store\n",
    )?;

    // tsconfig.json — enables TypeScript and IDE support.
    // Rex extracts @types/react and @limlabs/rex into node_modules on first run.
    std::fs::write(
        project_dir.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "noEmit": true,
    "paths": {
      "rex/*": ["./node_modules/@limlabs/rex/src/*"]
    }
  },
  "include": ["pages/**/*"]
}
"#,
    )?;

    // pages/index.tsx
    std::fs::write(
        project_dir.join("pages/index.tsx"),
        r#"export default function Home() {
  return (
    <div style={{ fontFamily: "system-ui, sans-serif", padding: "2rem", maxWidth: "640px" }}>
      <h1>Welcome to Rex</h1>
      <p>Edit <code>pages/index.tsx</code> to get started.</p>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      createdAt: new Date().toISOString(),
    },
  };
}
"#,
    )?;

    eprintln!("  {} {}", green_bold("✓"), green_bold("Project created"));
    eprintln!();
    eprintln!("  {}", dim("Get started:"));
    eprintln!();
    eprintln!("    {} {}", bold("cd"), bold(&name));
    eprintln!("    {} {}", bold("rex dev"), dim(""));
    eprintln!();

    Ok(())
}

#[cfg(feature = "lint")]
fn cmd_lint(root: PathBuf, fix: bool, deny_warnings: bool, paths: Vec<PathBuf>) -> Result<()> {
    use oxc_linter::{
        ConfigStore, ConfigStoreBuilder, ExternalPluginStore, FixKind, LintOptions, LintRunner,
        LintServiceOptions, Linter, Oxlintrc,
    };
    use std::ffi::OsStr;
    use std::sync::Arc;

    let root = std::fs::canonicalize(&root)?;

    // Load config: .oxlintrc.json if present, otherwise use Rex defaults
    let config_path = root.join(".oxlintrc.json");
    let oxlintrc = if config_path.exists() {
        Oxlintrc::from_file(&config_path)
            .map_err(|e| anyhow::anyhow!("failed to parse .oxlintrc.json: {e}"))?
    } else {
        Oxlintrc::from_string(default_oxlintrc())
            .map_err(|e| anyhow::anyhow!("failed to parse default oxlintrc: {e}"))?
    };

    // Build config store
    let mut external_plugin_store = ExternalPluginStore::default();
    let config_builder =
        ConfigStoreBuilder::from_oxlintrc(false, oxlintrc, None, &mut external_plugin_store, None)
            .map_err(|e| anyhow::anyhow!("failed to build lint config: {e}"))?;

    let base_config = config_builder
        .build(&mut external_plugin_store)
        .map_err(|e| anyhow::anyhow!("failed to build lint config: {e}"))?;

    let config_store = ConfigStore::new(base_config, Default::default(), external_plugin_store);

    // Create linter
    let fix_kind = if fix { FixKind::SafeFix } else { FixKind::None };
    let linter = Linter::new(LintOptions::default(), config_store, None).with_fix(fix_kind);

    // Determine lint targets
    let lint_dirs: Vec<PathBuf> = if paths.is_empty() {
        let pages_dir = root.join("pages");
        if pages_dir.is_dir() {
            vec![pages_dir]
        } else {
            vec![root.clone()]
        }
    } else {
        paths
            .into_iter()
            .map(|p| if p.is_absolute() { p } else { root.join(p) })
            .collect()
    };

    // Discover source files (respecting .gitignore + hardcoded skip dirs)
    let gitignore = load_gitignore_patterns(&root);
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in &lint_dirs {
        if dir.is_file() {
            files.push(dir.clone());
        } else if dir.is_dir() {
            walk_lint_dir(dir, &root, &gitignore, &mut files);
        }
    }

    if files.is_empty() {
        eprintln!();
        eprintln!("  {} {}", magenta_bold("◆ rex lint"), dim("(oxlint)"));
        eprintln!();
        eprintln!("  {} {}", dim("No source files found to lint"), dim(""));
        eprintln!();
        return Ok(());
    }

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex lint"), dim("(oxlint)"));
    eprintln!();

    // Build LintRunner and execute
    let service_options = LintServiceOptions::new(root.clone().into_boxed_path());

    let lint_runner = LintRunner::builder(service_options, linter)
        .with_fix_kind(fix_kind)
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build lint runner: {e}"))?;

    let file_paths: Vec<Arc<OsStr>> = files
        .iter()
        .map(|p| Arc::from(p.as_os_str().to_owned()))
        .collect();

    let (tx, rx) = std::sync::mpsc::channel::<Vec<oxc_diagnostics::Error>>();
    let lint_runner = lint_runner
        .lint_files(&file_paths, tx)
        .map_err(|e| anyhow::anyhow!("lint failed: {e}"))?;

    // Collect and display diagnostics
    let mut error_count: usize = 0;
    let mut warning_count: usize = 0;

    let _ = &lint_runner; // keep runner alive for fix writing

    for errors in rx {
        for error in &errors {
            let severity = error.severity().unwrap_or(oxc_diagnostics::Severity::Error);
            match severity {
                oxc_diagnostics::Severity::Error => error_count += 1,
                oxc_diagnostics::Severity::Warning => warning_count += 1,
                _ => {}
            }
            // Print diagnostics using miette-style formatting
            eprintln!("{error:?}");
        }
    }

    let total = error_count + warning_count;

    if total == 0 {
        eprintln!("  {} {}", green_bold("✓"), green_bold("No lint errors"));
        eprintln!();
        return Ok(());
    }

    eprintln!();
    if error_count > 0 {
        eprintln!(
            "  {} {}",
            bold(&format!("{error_count} error(s)")),
            if warning_count > 0 {
                format!("and {} warning(s)", warning_count)
            } else {
                String::new()
            }
        );
    } else {
        eprintln!("  {}", bold(&format!("{warning_count} warning(s)")));
    }
    eprintln!();

    if error_count > 0 || (deny_warnings && warning_count > 0) {
        std::process::exit(1);
    }

    Ok(())
}

/// Walk a directory recursively, collecting lintable source files.
/// Hardcoded directories that should always be skipped during linting,
/// even when no `.gitignore` is present.
#[cfg(feature = "lint")]
const LINT_SKIP_DIRS: &[&str] = &["node_modules", ".rex", ".git", "dist", "target", ".next"];

#[cfg(feature = "lint")]
fn walk_lint_dir(
    dir: &std::path::Path,
    root: &std::path::Path,
    gitignore: &[String],
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if LINT_SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
                continue;
            }
            if !gitignore.is_empty() && is_ignored(&path, root, gitignore) {
                continue;
            }
            walk_lint_dir(&path, root, gitignore, out);
        } else if path.is_file() {
            if let Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "mts") =
                path.extension().and_then(|e| e.to_str())
            {
                // Skip .d.ts type definition files
                if !path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(".d.ts"))
                {
                    if !gitignore.is_empty() && is_ignored(&path, root, gitignore) {
                        continue;
                    }
                    out.push(path);
                }
            }
        }
    }
}

/// Load ignore patterns from `.gitignore` (if present).
#[cfg(feature = "lint")]
fn load_gitignore_patterns(root: &std::path::Path) -> Vec<String> {
    let gitignore_path = root.join(".gitignore");
    let content = match std::fs::read_to_string(&gitignore_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        // Strip leading "/" — .gitignore uses it for root-relative, our is_ignored already does prefix matching
        .map(|l| l.strip_prefix('/').unwrap_or(l).to_string())
        .collect()
}

#[cfg(feature = "dev")]
fn cmd_typecheck(root: PathBuf, extra_args: Vec<String>) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;

    let tsc = rex_dev::typecheck::find_tsc(&root).ok_or_else(|| {
        anyhow::anyhow!(
            "tsc not found. Install TypeScript:\n\n  \
             npm install -D typescript\n  \
             # or globally: npm install -g typescript"
        )
    })?;

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex typecheck"), dim("(tsc)"));
    eprintln!();

    let mut cmd = Command::new(&tsc);
    cmd.current_dir(&root);
    cmd.arg("--noEmit");

    for arg in &extra_args {
        cmd.arg(arg);
    }

    let status = cmd.status()?;

    if status.success() {
        eprintln!();
        eprintln!("  {} {}", green_bold("✓"), green_bold("No type errors"));
        eprintln!();
    }

    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(feature = "lint")]
fn cmd_fmt(root: PathBuf, check: bool) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let options = load_format_options(&root);
    let ignore_patterns = load_ignore_patterns(&root);
    let mut files = discover_source_files(&root);

    if !ignore_patterns.is_empty() {
        files.retain(|f| !is_ignored(f, &root, &ignore_patterns));
    }

    if files.is_empty() {
        eprintln!();
        eprintln!("  {} {}", dim("◆ rex fmt"), dim("(oxfmt)"));
        eprintln!();
        eprintln!(
            "  {} {}",
            dim("No source files found in"),
            dim(&root.display().to_string())
        );
        eprintln!();
        return Ok(());
    }

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex fmt"), dim("(oxfmt)"));
    eprintln!();

    let changed_count = AtomicUsize::new(0);
    let error_count = AtomicUsize::new(0);
    let unformatted: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

    files.par_iter().for_each(|path| {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("  {} {}: {e}", dim("skip"), path.display());
                error_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        let formatted = match format_source(&source, path, &options) {
            Ok(f) => f,
            Err(_) => {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                eprintln!("  {} {} (parse error)", dim("skip"), rel.display());
                error_count.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        if formatted != source {
            if check {
                if let Ok(mut list) = unformatted.lock() {
                    list.push(path.clone());
                }
            } else {
                match std::fs::write(path, &formatted) {
                    Ok(()) => {
                        changed_count.fetch_add(1, Ordering::Relaxed);
                        let rel = path.strip_prefix(&root).unwrap_or(path);
                        eprintln!("  {} {}", dim("fmt"), rel.display());
                    }
                    Err(e) => {
                        eprintln!("  {} {}: {e}", dim("error"), path.display());
                        error_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    });

    let changed = changed_count.load(Ordering::Relaxed);
    let errors = error_count.load(Ordering::Relaxed);

    if check {
        let unformatted = unformatted.into_inner().unwrap_or_default();
        if unformatted.is_empty() {
            eprintln!(
                "  {} {}",
                green_bold("✓"),
                green_bold("All files formatted")
            );
            eprintln!();
            Ok(())
        } else {
            for path in &unformatted {
                let rel = path.strip_prefix(&root).unwrap_or(path);
                eprintln!("  {} {}", dim("unformatted"), rel.display());
            }
            eprintln!();
            eprintln!(
                "  {} {}",
                bold(&format!("{} file(s) need formatting", unformatted.len())),
                dim("(run `rex fmt` to fix)")
            );
            eprintln!();
            std::process::exit(1);
        }
    } else {
        if changed == 0 && errors == 0 {
            eprintln!(
                "  {} {}",
                green_bold("✓"),
                green_bold("All files formatted")
            );
        } else if changed > 0 {
            eprintln!();
            eprintln!(
                "  {} {}",
                green_bold("✓"),
                green_bold(&format!("Formatted {changed} file(s)"))
            );
        }
        if errors > 0 {
            eprintln!(
                "  {} {}",
                dim("⚠"),
                dim(&format!("{errors} file(s) skipped"))
            );
        }
        eprintln!();
        Ok(())
    }
}

#[cfg(feature = "lint")]
fn discover_source_files(root: &std::path::Path) -> Vec<PathBuf> {
    let extensions: &[&str] = &["ts", "tsx", "js", "jsx"];
    let skip_dirs: &[&str] = &["node_modules", ".rex", ".git", "dist", "target", ".next"];

    let mut files = Vec::new();

    // Scan pages/ and styles/ directories
    let scan_dirs = ["pages", "styles", "components", "lib", "utils", "src"];
    for dir_name in &scan_dirs {
        let dir = root.join(dir_name);
        if dir.is_dir() {
            walk_dir(&dir, extensions, skip_dirs, &mut files);
        }
    }

    // Also pick up root-level config files (e.g., next.config.js)
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.contains(&ext) {
                        files.push(path);
                    }
                }
            }
        }
    }

    files.sort();
    files.dedup();
    files
}

#[cfg(feature = "lint")]
fn walk_dir(
    dir: &std::path::Path,
    extensions: &[&str],
    skip_dirs: &[&str],
    files: &mut Vec<PathBuf>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !skip_dirs.contains(&name) {
                walk_dir(&path, extensions, skip_dirs, files);
            }
        } else if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    files.push(path);
                }
            }
        }
    }
}

#[cfg(feature = "lint")]
fn default_oxlintrc() -> &'static str {
    r#"{
  "$schema": "https://raw.githubusercontent.com/oxc-project/oxc/main/npm/oxlint/configuration_schema.json",
  "plugins": ["react", "react-hooks", "nextjs", "import"],
  "rules": {
    "react/jsx-no-target-blank": "warn",
    "react/no-unknown-property": "warn",
    "react/react-in-jsx-scope": "off",
    "react-hooks/rules-of-hooks": "error",
    "react-hooks/exhaustive-deps": "warn",
    "nextjs/no-html-link-for-pages": "warn",
    "nextjs/no-img-element": "warn",
    "nextjs/no-head-import-in-document": "warn",
    "nextjs/no-duplicate-head": "warn",
    "import/no-cycle": "warn",
    "no-var": "error"
  },
  "ignorePatterns": [".rex/", "node_modules/"]
}
"#
}

// --- Display helpers ---

fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[0m")
}

fn dim(s: &str) -> String {
    format!("\x1b[2m{s}\x1b[0m")
}

fn magenta_bold(s: &str) -> String {
    format!("\x1b[1;35m{s}\x1b[0m")
}

fn green_bold(s: &str) -> String {
    format!("\x1b[1;32m{s}\x1b[0m")
}

fn emerald(s: &str) -> String {
    format!("\x1b[38;2;46;204;113m{s}\x1b[0m")
}

fn print_mascot_header(version: &str, suffix: &str) {
    // Rex mascot - each line padded to 20 display chars
    let m: [&str; 5] = [
        "        ▄████▄       ",
        "        █ ◦ █▀█▄    ",
        "  ▄▄▄▄▄▄█████▀▀     ",
        "    ▀▀▀▀██████       ",
        "        █▀ █▀        ",
    ];
    let version_line = if suffix.is_empty() {
        format!("{} {}", emerald("rex"), dim(version))
    } else {
        format!("{} {} {}", emerald("rex"), dim(version), dim(suffix))
    };
    eprintln!();
    eprintln!("  {}", emerald(m[0]));
    eprintln!("  {}  {}", emerald(m[1]), version_line);
    eprintln!("  {}", emerald(m[2]));
    eprintln!("  {}", emerald(m[3]));
    eprintln!("  {}", emerald(m[4]));
    eprintln!();
}

fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms >= 1000 {
        format!("{:.2}s", d.as_secs_f64())
    } else {
        format!("{ms}ms")
    }
}

#[cfg(feature = "build")]
fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn print_route_summary(routes: &[rex_core::Route], api_routes: &[rex_core::Route]) {
    let page_count = routes.len();
    let api_count = api_routes.len();

    let mut parts = Vec::new();
    if page_count > 0 {
        parts.push(format!(
            "{} {}",
            page_count,
            if page_count == 1 { "page" } else { "pages" }
        ));
    }
    if api_count > 0 {
        parts.push(format!(
            "{} API {}",
            api_count,
            if api_count == 1 { "route" } else { "routes" }
        ));
    }

    if !parts.is_empty() {
        eprintln!("  {}", dim(&parts.join(" · ")));
    }
}

/// Classified route info for build summary display.
#[cfg(feature = "build")]
struct RouteInfo {
    pattern: String,
    icon: &'static str,
    label: &'static str,
}

/// Classify routes by render mode and count static vs. server-rendered pages.
///
/// Returns (route_infos, static_count, dynamic_count) sorted by pattern.
#[cfg(feature = "build")]
fn classify_routes(
    routes: &[rex_core::Route],
    manifest: &AssetManifest,
) -> (Vec<RouteInfo>, usize, usize) {
    use rex_core::RenderMode;

    let mut sorted: Vec<_> = routes.iter().collect();
    sorted.sort_by(|a, b| a.pattern.cmp(&b.pattern));

    let mut static_count = 0usize;
    let mut dynamic_count = 0usize;
    let mut infos = Vec::with_capacity(sorted.len());

    for route in &sorted {
        let render_mode = manifest
            .pages
            .get(&route.pattern)
            .map(|p| p.render_mode)
            .unwrap_or(RenderMode::ServerRendered);

        let (icon, label) = match render_mode {
            RenderMode::Static => {
                static_count += 1;
                ("\u{25cb}", "static") // ○
            }
            RenderMode::ServerRendered => {
                dynamic_count += 1;
                ("\u{03bb}", "server") // λ
            }
        };

        infos.push(RouteInfo {
            pattern: route.pattern.clone(),
            icon,
            label,
        });
    }

    (infos, static_count, dynamic_count)
}

/// Print route summary with static/dynamic indicators (used by `rex build`).
///
/// Like Next.js, shows:
///   ○  /about           (static)
///   λ  /                (server-rendered)
///   λ  /blog/:slug      (server-rendered)
/// Classify app routes into static/server-rendered categories.
#[cfg(feature = "build")]
fn classify_app_routes(manifest: &AssetManifest) -> (Vec<RouteInfo>, usize, usize) {
    use rex_core::RenderMode;

    let mut sorted: Vec<_> = manifest.app_routes.keys().collect();
    sorted.sort();

    let mut static_count = 0usize;
    let mut dynamic_count = 0usize;
    let mut infos = Vec::with_capacity(sorted.len());

    for pattern in &sorted {
        let render_mode = manifest
            .app_routes
            .get(*pattern)
            .map(|a| a.render_mode)
            .unwrap_or(RenderMode::ServerRendered);

        let (icon, label) = match render_mode {
            RenderMode::Static => {
                static_count += 1;
                ("\u{25cb}", "static") // ○
            }
            RenderMode::ServerRendered => {
                dynamic_count += 1;
                ("\u{03bb}", "server") // λ
            }
        };

        infos.push(RouteInfo {
            pattern: (*pattern).clone(),
            icon,
            label,
        });
    }

    (infos, static_count, dynamic_count)
}

#[cfg(feature = "build")]
fn print_route_summary_with_manifest(
    routes: &[rex_core::Route],
    api_routes: &[rex_core::Route],
    manifest: &AssetManifest,
) {
    if routes.is_empty() && api_routes.is_empty() && manifest.app_routes.is_empty() {
        return;
    }

    let (infos, mut static_count, mut dynamic_count) = classify_routes(routes, manifest);

    // Pages router routes
    for info in &infos {
        eprintln!(
            "    {} {} {}",
            dim(info.icon),
            dim(&format!("{:<30}", info.pattern)),
            dim(&format!("({})", info.label))
        );
    }

    // App router routes
    if !manifest.app_routes.is_empty() {
        let (app_infos, app_static, app_dynamic) = classify_app_routes(manifest);
        static_count += app_static;
        dynamic_count += app_dynamic;

        for info in &app_infos {
            eprintln!(
                "    {} {} {}",
                dim(info.icon),
                dim(&format!("{:<30}", info.pattern)),
                dim(&format!("({})", info.label))
            );
        }
    }

    for route in api_routes {
        eprintln!(
            "    {} {} {}",
            dim("\u{03bb}"),
            dim(&format!("{:<30}", route.pattern)),
            dim("(api)")
        );
    }

    eprintln!();

    let mut legend = Vec::new();
    if static_count > 0 {
        legend.push(format!("\u{25cb} static ({static_count})"));
    }
    if dynamic_count > 0 {
        legend.push(format!("\u{03bb} server ({dynamic_count})"));
    }
    if !api_routes.is_empty() {
        legend.push(format!("\u{03bb} api ({})", api_routes.len()));
    }
    if !legend.is_empty() {
        eprintln!("  {}", dim(&legend.join("  ")));
    }
}

#[cfg(feature = "live")]
async fn cmd_live(mount: Vec<String>, port: u16, host: IpAddr, workers: usize) -> Result<()> {
    use rex_live::server::{LiveServerConfig, MountConfig};

    let start = std::time::Instant::now();

    print_mascot_header(env!("CARGO_PKG_VERSION"), "(live)");

    if mount.is_empty() {
        anyhow::bail!("No mounts specified. Use --mount PREFIX=SOURCE (e.g., --mount /=./my-app)");
    }

    let mut mounts = Vec::new();
    for m in &mount {
        let (prefix, source) = m
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid mount format: {m}. Expected PREFIX=SOURCE"))?;

        let source_path = std::fs::canonicalize(source)
            .map_err(|e| anyhow::anyhow!("Source path not found: {source}: {e}"))?;

        mounts.push(MountConfig {
            prefix: prefix.to_string(),
            source: source_path,
        });
    }

    let config = LiveServerConfig {
        mounts,
        port,
        host,
        workers_per_project: workers,
    };

    let server = rex_live::server::LiveServer::new(&config)?;

    let elapsed = start.elapsed();
    eprintln!(
        "  {} {}",
        green_bold("✓ Ready in"),
        green_bold(&format_duration(elapsed))
    );
    eprintln!();
    eprintln!(
        "  {} {}",
        dim("➜ Local:"),
        bold(&format!("http://localhost:{port}"))
    );
    eprintln!();
    for project in server.projects() {
        eprintln!(
            "  {} {} → {}",
            dim("◇"),
            bold(&project.prefix),
            dim(&project.source_root().display().to_string()),
        );
    }
    eprintln!();
    eprintln!(
        "  {}",
        dim("Projects compile on first request. File changes auto-invalidate cache.")
    );
    eprintln!();

    server.serve(port, host).await
}

async fn cmd_start(root: PathBuf, port: u16, host: IpAddr) -> Result<()> {
    let start = std::time::Instant::now();

    print_mascot_header(env!("CARGO_PKG_VERSION"), "(production)");

    let rex = rex_server::Rex::from_build(rex_server::RexOptions {
        root,
        dev: false,
        port,
        host,
    })
    .await?;

    let elapsed = start.elapsed();
    eprintln!(
        "  {} {}",
        green_bold("✓ Ready in"),
        green_bold(&format_duration(elapsed))
    );
    eprintln!();
    eprintln!(
        "  {} {}",
        dim("➜ Local:"),
        bold(&format!("http://localhost:{port}"))
    );
    eprintln!();
    print_route_summary(&rex.scan().routes, &rex.scan().api_routes);
    eprintln!();

    rex.serve().await
}

#[cfg(feature = "dev")]
async fn hmr_client_handler() -> impl axum::response::IntoResponse {
    let js = include_str!(concat!(env!("OUT_DIR"), "/hmr_client.js"));
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}

#[cfg(feature = "lint")]
fn load_format_options(root: &std::path::Path) -> oxc_formatter::FormatOptions {
    let config_files = [".prettierrc", ".prettierrc.json"];

    let json_value: Option<serde_json::Value> = config_files
        .iter()
        .find_map(|name| {
            let path = root.join(name);
            let content = std::fs::read_to_string(&path).ok()?;
            serde_json::from_str(&content).ok()
        })
        .or_else(|| {
            let pkg_path = root.join("package.json");
            let content = std::fs::read_to_string(&pkg_path).ok()?;
            let pkg: serde_json::Value = serde_json::from_str(&content).ok()?;
            pkg.get("prettier").cloned()
        });

    let Some(config) = json_value else {
        return oxc_formatter::FormatOptions {
            quote_style: oxc_formatter::QuoteStyle::Single,
            ..Default::default()
        };
    };

    let mut options = oxc_formatter::FormatOptions::default();

    if let Some(v) = config.get("singleQuote").and_then(|v| v.as_bool()) {
        options.quote_style = if v {
            oxc_formatter::QuoteStyle::Single
        } else {
            oxc_formatter::QuoteStyle::Double
        };
    }

    if let Some(v) = config.get("jsxSingleQuote").and_then(|v| v.as_bool()) {
        options.jsx_quote_style = if v {
            oxc_formatter::QuoteStyle::Single
        } else {
            oxc_formatter::QuoteStyle::Double
        };
    }

    if let Some(v) = config.get("tabWidth").and_then(|v| v.as_u64()) {
        if let Ok(w) = oxc_formatter::IndentWidth::try_from(v as u8) {
            options.indent_width = w;
        }
    }

    if let Some(v) = config.get("useTabs").and_then(|v| v.as_bool()) {
        options.indent_style = if v {
            oxc_formatter::IndentStyle::Tab
        } else {
            oxc_formatter::IndentStyle::Space
        };
    }

    if let Some(v) = config.get("printWidth").and_then(|v| v.as_u64()) {
        if let Ok(w) = oxc_formatter::LineWidth::try_from(v as u16) {
            options.line_width = w;
        }
    }

    if let Some(v) = config.get("semi").and_then(|v| v.as_bool()) {
        options.semicolons = if v {
            oxc_formatter::Semicolons::Always
        } else {
            oxc_formatter::Semicolons::AsNeeded
        };
    }

    if let Some(v) = config.get("trailingComma").and_then(|v| v.as_str()) {
        options.trailing_commas = match v {
            "all" => oxc_formatter::TrailingCommas::All,
            "none" => oxc_formatter::TrailingCommas::None,
            "es5" => oxc_formatter::TrailingCommas::Es5,
            _ => options.trailing_commas,
        };
    }

    if let Some(v) = config.get("bracketSpacing").and_then(|v| v.as_bool()) {
        options.bracket_spacing = oxc_formatter::BracketSpacing::from(v);
    }

    if let Some(v) = config.get("bracketSameLine").and_then(|v| v.as_bool()) {
        options.bracket_same_line = oxc_formatter::BracketSameLine::from(v);
    }

    if let Some(v) = config.get("arrowParens").and_then(|v| v.as_str()) {
        options.arrow_parentheses = match v {
            "avoid" => oxc_formatter::ArrowParentheses::AsNeeded,
            "always" => oxc_formatter::ArrowParentheses::Always,
            _ => options.arrow_parentheses,
        };
    }

    if let Some(v) = config.get("endOfLine").and_then(|v| v.as_str()) {
        options.line_ending = match v {
            "lf" => oxc_formatter::LineEnding::Lf,
            "crlf" => oxc_formatter::LineEnding::Crlf,
            "cr" => oxc_formatter::LineEnding::Cr,
            _ => options.line_ending,
        };
    }

    options
}

#[cfg(feature = "lint")]
fn load_ignore_patterns(root: &std::path::Path) -> Vec<String> {
    let ignore_path = root.join(".prettierignore");
    let content = match std::fs::read_to_string(&ignore_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

#[cfg(feature = "lint")]
fn is_ignored(path: &std::path::Path, root: &std::path::Path, patterns: &[String]) -> bool {
    let rel = match path.strip_prefix(root) {
        Ok(r) => r.to_string_lossy(),
        Err(_) => return false,
    };
    let rel_str = rel.as_ref();

    for pattern in patterns {
        // Directory pattern (e.g., "dist/", "pages/api/")
        if let Some(dir) = pattern.strip_suffix('/') {
            if rel_str.starts_with(dir) || rel_str.starts_with(&format!("{dir}/")) {
                return true;
            }
        }
        // Glob-like extension pattern (e.g., "*.min.js")
        else if let Some(suffix) = pattern.strip_prefix('*') {
            if rel_str.ends_with(suffix) {
                return true;
            }
        }
        // Exact match or prefix match for bare directory names (e.g. "dist" matches "dist/foo.ts")
        else if rel_str == pattern || rel_str.starts_with(&format!("{pattern}/")) {
            return true;
        }
    }
    false
}

#[cfg(feature = "lint")]
fn format_source(
    source: &str,
    path: &std::path::Path,
    options: &oxc_formatter::FormatOptions,
) -> Result<String> {
    let source_type = oxc_span::SourceType::from_path(path)
        .map_err(|e| anyhow::anyhow!("unsupported file type: {e}"))?;
    let allocator = oxc_allocator::Allocator::default();
    let parse_options = oxc_parser::ParseOptions {
        preserve_parens: false,
        ..Default::default()
    };
    let parsed = oxc_parser::Parser::new(&allocator, source, source_type)
        .with_options(parse_options)
        .parse();
    if !parsed.errors.is_empty() {
        anyhow::bail!("parse error");
    }
    Ok(oxc_formatter::Formatter::new(&allocator, options.clone()).build(&parsed.program))
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
        let value = value.trim();
        // Strip surrounding quotes
        let value = if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
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
    use std::path::Path;

    fn default_options() -> oxc_formatter::FormatOptions {
        oxc_formatter::FormatOptions {
            quote_style: oxc_formatter::QuoteStyle::Single,
            ..Default::default()
        }
    }

    #[test]
    fn test_format_source_single_quotes() {
        let input = "const x = \"hello\";\n";
        let opts = default_options();
        let result = format_source(input, Path::new("test.ts"), &opts).unwrap();
        assert!(
            result.contains("'hello'"),
            "expected single quotes, got: {result}"
        );
    }

    #[test]
    fn test_format_source_semicolons() {
        let input = "const x = 1\n";
        let opts = default_options();
        let result = format_source(input, Path::new("test.ts"), &opts).unwrap();
        assert!(
            result.contains("const x = 1;"),
            "expected semicolons, got: {result}"
        );
    }

    #[test]
    fn test_format_source_tsx() {
        let input = "export default function App() { return <div>hi</div>; }\n";
        let opts = default_options();
        let result = format_source(input, Path::new("test.tsx"), &opts).unwrap();
        assert!(result.contains("<div>"), "expected JSX preserved: {result}");
    }

    #[test]
    fn test_format_source_idempotent() {
        let input = "const x = 'hello';\n";
        let opts = default_options();
        let first = format_source(input, Path::new("test.ts"), &opts).unwrap();
        let second = format_source(&first, Path::new("test.ts"), &opts).unwrap();
        assert_eq!(first, second, "formatting should be idempotent");
    }

    #[test]
    fn test_format_source_parse_error() {
        let input = "const = ;;\n";
        let opts = default_options();
        let result = format_source(input, Path::new("test.ts"), &opts);
        assert!(result.is_err(), "should fail on invalid syntax");
    }

    #[test]
    fn test_discover_source_files_finds_pages() {
        let tmp = tempfile::tempdir().unwrap();
        let pages = tmp.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("index.tsx"), "export default function() {}").unwrap();
        std::fs::write(pages.join("readme.md"), "# hello").unwrap();

        let files = discover_source_files(tmp.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("index.tsx"));
    }

    #[test]
    fn test_discover_source_files_skips_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let nm = tmp.path().join("pages/node_modules/foo");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("bar.ts"), "const x = 1").unwrap();

        let files = discover_source_files(tmp.path());
        assert!(files.is_empty(), "should skip node_modules");
    }

    #[test]
    fn test_discover_source_files_root_configs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("next.config.js"), "module.exports = {}").unwrap();
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();

        let files = discover_source_files(tmp.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("next.config.js"));
    }

    #[test]
    fn test_walk_dir_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.ts"), "").unwrap();
        std::fs::write(tmp.path().join("b.tsx"), "").unwrap();
        std::fs::write(tmp.path().join("c.css"), "").unwrap();
        std::fs::write(tmp.path().join("d.js"), "").unwrap();

        let extensions: &[&str] = &["ts", "tsx", "js", "jsx"];
        let skip_dirs: &[&str] = &["node_modules"];
        let mut files = Vec::new();
        walk_dir(tmp.path(), extensions, skip_dirs, &mut files);

        assert_eq!(files.len(), 3, "should find .ts, .tsx, .js but not .css");
    }

    #[test]
    fn test_walk_dir_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.ts"), "").unwrap();

        let extensions: &[&str] = &["ts"];
        let skip_dirs: &[&str] = &[];
        let mut files = Vec::new();
        walk_dir(tmp.path(), extensions, skip_dirs, &mut files);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("deep.ts"));
    }

    #[test]
    fn test_cmd_fmt_write_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let pages = tmp.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("index.ts"), "const x = \"hello\"\n").unwrap();

        cmd_fmt(tmp.path().to_path_buf(), false).unwrap();

        let content = std::fs::read_to_string(pages.join("index.ts")).unwrap();
        assert!(
            content.contains("'hello'"),
            "should have formatted to single quotes: {content}"
        );
        assert!(
            content.contains(';'),
            "should have added semicolons: {content}"
        );
    }

    #[test]
    fn test_cmd_fmt_check_mode_passes_when_formatted() {
        let tmp = tempfile::tempdir().unwrap();
        let pages = tmp.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();

        let opts = default_options();
        let formatted = format_source("const x = \"hello\";\n", Path::new("t.ts"), &opts).unwrap();
        std::fs::write(pages.join("index.ts"), &formatted).unwrap();

        cmd_fmt(tmp.path().to_path_buf(), true).unwrap();
    }

    #[test]
    fn test_cmd_fmt_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        cmd_fmt(tmp.path().to_path_buf(), false).unwrap();
    }

    #[test]
    fn test_cmd_fmt_skips_parse_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let pages = tmp.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("broken.ts"), "const = ;;\n").unwrap();
        std::fs::write(pages.join("good.ts"), "const x = \"hello\"\n").unwrap();

        cmd_fmt(tmp.path().to_path_buf(), false).unwrap();

        let broken = std::fs::read_to_string(pages.join("broken.ts")).unwrap();
        assert_eq!(broken, "const = ;;\n");

        let good = std::fs::read_to_string(pages.join("good.ts")).unwrap();
        assert!(good.contains("'hello'"));
    }

    #[test]
    fn test_discover_multiple_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("pages")).unwrap();
        std::fs::create_dir_all(tmp.path().join("components")).unwrap();
        std::fs::create_dir_all(tmp.path().join("lib")).unwrap();
        std::fs::write(tmp.path().join("pages/index.tsx"), "").unwrap();
        std::fs::write(tmp.path().join("components/btn.tsx"), "").unwrap();
        std::fs::write(tmp.path().join("lib/utils.ts"), "").unwrap();

        let files = discover_source_files(tmp.path());
        assert_eq!(
            files.len(),
            3,
            "should find files in pages, components, lib"
        );
    }

    // --- Prettier config tests ---

    #[test]
    fn test_load_format_options_prettierrc() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".prettierrc"),
            r#"{ "singleQuote": false }"#,
        )
        .unwrap();

        let opts = load_format_options(tmp.path());
        assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Double);
    }

    #[test]
    fn test_load_format_options_prettierrc_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".prettierrc.json"), r#"{ "tabWidth": 4 }"#).unwrap();

        let opts = load_format_options(tmp.path());
        assert_eq!(opts.indent_width.value(), 4);
    }

    #[test]
    fn test_load_format_options_package_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{ "name": "test", "prettier": { "singleQuote": true, "tabWidth": 4 } }"#,
        )
        .unwrap();

        let opts = load_format_options(tmp.path());
        assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Single);
        assert_eq!(opts.indent_width.value(), 4);
    }

    #[test]
    fn test_load_format_options_precedence() {
        let tmp = tempfile::tempdir().unwrap();
        // .prettierrc should win over package.json
        std::fs::write(tmp.path().join(".prettierrc"), r#"{ "singleQuote": true }"#).unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{ "prettier": { "singleQuote": false } }"#,
        )
        .unwrap();

        let opts = load_format_options(tmp.path());
        assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Single);
    }

    #[test]
    fn test_load_format_options_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        // No config files at all
        let opts = load_format_options(tmp.path());
        assert_eq!(
            opts.quote_style,
            oxc_formatter::QuoteStyle::Single,
            "should default to single quotes"
        );
        assert_eq!(
            opts.indent_width.value(),
            2,
            "should default to 2-space indent"
        );
    }

    #[test]
    fn test_load_format_options_all_fields() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(".prettierrc"),
            r#"{
                "singleQuote": true,
                "jsxSingleQuote": true,
                "tabWidth": 4,
                "useTabs": true,
                "printWidth": 120,
                "semi": false,
                "trailingComma": "none",
                "bracketSpacing": false,
                "bracketSameLine": true,
                "arrowParens": "avoid",
                "endOfLine": "crlf"
            }"#,
        )
        .unwrap();

        let opts = load_format_options(tmp.path());
        assert_eq!(opts.quote_style, oxc_formatter::QuoteStyle::Single);
        assert_eq!(opts.jsx_quote_style, oxc_formatter::QuoteStyle::Single);
        assert_eq!(opts.indent_width.value(), 4);
        assert_eq!(opts.indent_style, oxc_formatter::IndentStyle::Tab);
        assert_eq!(opts.line_width.value(), 120);
        assert_eq!(opts.semicolons, oxc_formatter::Semicolons::AsNeeded);
        assert_eq!(opts.trailing_commas, oxc_formatter::TrailingCommas::None);
        assert!(!opts.bracket_spacing.value());
        assert!(opts.bracket_same_line.value());
        assert_eq!(
            opts.arrow_parentheses,
            oxc_formatter::ArrowParentheses::AsNeeded
        );
        assert_eq!(opts.line_ending, oxc_formatter::LineEnding::Crlf);
    }

    #[test]
    fn test_load_ignore_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let pages = tmp.path().join("pages");
        let api = pages.join("api");
        std::fs::create_dir_all(&api).unwrap();
        std::fs::write(pages.join("index.ts"), "const x = 1").unwrap();
        std::fs::write(api.join("hello.ts"), "const y = 2").unwrap();

        std::fs::write(
            tmp.path().join(".prettierignore"),
            "# comment\npages/api/\n",
        )
        .unwrap();

        let patterns = load_ignore_patterns(tmp.path());
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0], "pages/api/");

        let files = discover_source_files(tmp.path());
        let filtered: Vec<_> = files
            .into_iter()
            .filter(|f| !is_ignored(f, tmp.path(), &patterns))
            .collect();
        assert_eq!(filtered.len(), 1, "should filter out api files");
        assert!(filtered[0].ends_with("index.ts"));
    }

    #[test]
    fn test_format_source_with_options() {
        let input = "const x = 'hello';\n";
        let opts = oxc_formatter::FormatOptions {
            quote_style: oxc_formatter::QuoteStyle::Double,
            ..Default::default()
        };
        let result = format_source(input, Path::new("test.ts"), &opts).unwrap();
        assert!(
            result.contains("\"hello\""),
            "expected double quotes with custom options, got: {result}"
        );
    }

    fn make_route(
        pattern: &str,
        dynamic_segments: Vec<rex_core::DynamicSegment>,
    ) -> rex_core::Route {
        rex_core::Route {
            pattern: pattern.into(),
            file_path: format!("pages{pattern}.tsx").into(),
            abs_path: format!("/pages{pattern}.tsx").into(),
            page_type: rex_core::PageType::Regular,
            dynamic_segments,
            specificity: 0,
        }
    }

    #[test]
    fn classify_routes_static_and_dynamic() {
        use rex_core::DataStrategy;

        let routes = vec![
            make_route("/about", vec![]),
            make_route("/", vec![]),
            make_route(
                "/blog/:slug",
                vec![rex_core::DynamicSegment::Single("slug".into())],
            ),
        ];

        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/", "index.js", DataStrategy::None, false);
        manifest.add_page("/about", "about.js", DataStrategy::None, false);
        manifest.add_page("/blog/:slug", "slug.js", DataStrategy::None, true);

        let (infos, static_count, dynamic_count) = classify_routes(&routes, &manifest);

        // Sorted by pattern
        assert_eq!(infos[0].pattern, "/");
        assert_eq!(infos[1].pattern, "/about");
        assert_eq!(infos[2].pattern, "/blog/:slug");

        assert_eq!(static_count, 2);
        assert_eq!(dynamic_count, 1);

        assert_eq!(infos[0].label, "static");
        assert_eq!(infos[2].label, "server");
    }

    #[test]
    fn classify_routes_gssp_is_server() {
        use rex_core::DataStrategy;

        let routes = vec![make_route("/dashboard", vec![])];

        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page(
            "/dashboard",
            "dashboard.js",
            DataStrategy::GetServerSideProps,
            false,
        );

        let (infos, static_count, dynamic_count) = classify_routes(&routes, &manifest);

        assert_eq!(static_count, 0);
        assert_eq!(dynamic_count, 1);
        assert_eq!(infos[0].label, "server");
    }

    #[test]
    fn classify_routes_gsp_is_static() {
        use rex_core::DataStrategy;

        let routes = vec![make_route("/posts", vec![])];

        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/posts", "posts.js", DataStrategy::GetStaticProps, false);

        let (_, static_count, dynamic_count) = classify_routes(&routes, &manifest);

        assert_eq!(static_count, 1);
        assert_eq!(dynamic_count, 0);
    }

    #[test]
    fn classify_app_routes_static_and_dynamic() {
        use rex_core::{AppRouteAssets, RenderMode};

        let mut manifest = AssetManifest::new("test".into());
        manifest.app_routes.insert(
            "/".to_string(),
            AppRouteAssets {
                client_chunks: vec![],
                layout_chain: vec![],
                render_mode: RenderMode::Static,
            },
        );
        manifest.app_routes.insert(
            "/about".to_string(),
            AppRouteAssets {
                client_chunks: vec![],
                layout_chain: vec![],
                render_mode: RenderMode::Static,
            },
        );
        manifest.app_routes.insert(
            "/blog/:slug".to_string(),
            AppRouteAssets {
                client_chunks: vec![],
                layout_chain: vec![],
                render_mode: RenderMode::ServerRendered,
            },
        );

        let (infos, static_count, dynamic_count) = classify_app_routes(&manifest);

        assert_eq!(static_count, 2);
        assert_eq!(dynamic_count, 1);
        assert_eq!(infos.len(), 3);

        // Verify sorted order
        assert_eq!(infos[0].pattern, "/");
        assert_eq!(infos[1].pattern, "/about");
        assert_eq!(infos[2].pattern, "/blog/:slug");
    }

    #[test]
    fn classify_app_routes_empty() {
        let manifest = AssetManifest::new("test".into());
        let (infos, static_count, dynamic_count) = classify_app_routes(&manifest);
        assert!(infos.is_empty());
        assert_eq!(static_count, 0);
        assert_eq!(dynamic_count, 0);
    }
}
