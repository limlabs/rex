mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use rex_build::build_bundles;
use rex_core::{ProjectConfig, RexConfig};
use rex_router::scan_project;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
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
    Dev {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Disable TUI (use plain log output)
        #[arg(long)]
        no_tui: bool,
    },

    /// Create a production build
    Build {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },

    /// Start the production server
    Start {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },

    /// Lint pages with oxlint (React + Next.js rules)
    Lint {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Fix auto-fixable problems
        #[arg(long)]
        fix: bool,

        /// Additional arguments passed to oxlint
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Type-check pages with tsc
    Typecheck {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Additional arguments passed to tsc
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Create a new Rex project
    Init {
        /// Project name (creates a new directory)
        name: String,
    },

    /// Format source files with oxfmt
    Fmt {
        /// Project root directory
        #[arg(long, default_value = ".")]
        root: PathBuf,

        /// Check formatting without writing (exits with error if unformatted)
        #[arg(long)]
        check: bool,
    },
}

fn init_plain_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rex=info".into()),
        )
        .init();
}

fn init_tui_tracing() -> LogBuffer {
    let buffer = LogBuffer::new(1000);
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "rex=info".into());

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
        Commands::Dev { port, root, no_tui } => {
            let root = std::fs::canonicalize(&root)?;
            let project_config = ProjectConfig::load(&root)?;
            let is_terminal = std::io::IsTerminal::is_terminal(&std::io::stdout());
            let tui_enabled = !no_tui && !project_config.dev.no_tui && is_terminal;

            if tui_enabled {
                let log_buffer = init_tui_tracing();
                let log_buffer_fallback = log_buffer.clone();
                let result = cmd_dev(root, port, true, Some(log_buffer)).await;
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
                cmd_dev(root, port, false, None).await
            }
        }
        Commands::Build { root } => {
            init_plain_tracing();
            cmd_build(root).await
        }
        Commands::Start { port, root } => {
            init_plain_tracing();
            cmd_start(root, port).await
        }
        Commands::Lint { root, fix, args } => {
            init_plain_tracing();
            cmd_lint(root, fix, args)
        }
        Commands::Typecheck { root, args } => {
            init_plain_tracing();
            cmd_typecheck(root, args)
        }
        Commands::Init { name } => {
            init_plain_tracing();
            cmd_init(name)
        }
        Commands::Fmt { root, check } => {
            init_plain_tracing();
            cmd_fmt(root, check)
        }
    }
}

async fn cmd_dev(
    root: PathBuf,
    port: u16,
    tui_enabled: bool,
    log_buffer: Option<LogBuffer>,
) -> Result<()> {
    let start = std::time::Instant::now();

    // Bind the port early — fail fast on conflicts before the expensive build.
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind port {port} — is another server running?"))?;

    if !tui_enabled {
        print_mascot_header(env!("CARGO_PKG_VERSION"), "");
    }

    let rex = rex_server::Rex::new(rex_server::RexOptions {
        root: root.clone(),
        dev: true,
        port,
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
    let event_rx = rex_dev::start_watcher(&config.project_root, &config.pages_dir)?;

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

async fn cmd_build(root: PathBuf) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root);
    config.validate()?;

    print_mascot_header(env!("CARGO_PKG_VERSION"), "");

    let start = std::time::Instant::now();
    debug!("Building for production...");

    let scan = scan_project(&config.project_root, &config.pages_dir)?;
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
    print_route_summary(&scan.routes, &scan.api_routes);
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

    // Create directory structure
    std::fs::create_dir_all(project_dir.join("pages/api"))?;
    std::fs::create_dir_all(project_dir.join("styles"))?;
    std::fs::create_dir_all(project_dir.join("public"))?;

    // package.json
    std::fs::write(
        project_dir.join("package.json"),
        format!(
            r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "private": true,
  "dependencies": {{
    "react": "^19.0.0",
    "react-dom": "^19.0.0"
  }},
  "devDependencies": {{
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0"
  }}
}}
"#
        ),
    )?;

    // tsconfig.json
    std::fs::write(
        project_dir.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "target": "ESNext",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true
  },
  "include": ["pages/**/*"],
  "exclude": ["node_modules", ".rex"]
}
"#,
    )?;

    // .gitignore
    std::fs::write(
        project_dir.join(".gitignore"),
        "node_modules\n.rex\n.DS_Store\n",
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

    // pages/_app.tsx
    std::fs::write(
        project_dir.join("pages/_app.tsx"),
        r#"import '../styles/globals.css';

export default function App({ Component, pageProps }: { Component: any; pageProps: any }) {
  return <Component {...pageProps} />;
}
"#,
    )?;

    // pages/api/hello.ts
    std::fs::write(
        project_dir.join("pages/api/hello.ts"),
        r#"export default function handler(req: any, res: any) {
  res.status(200).json({ message: "Hello from Rex API!" });
}
"#,
    )?;

    // styles/globals.css
    std::fs::write(
        project_dir.join("styles/globals.css"),
        r#"*,
*::before,
*::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

body {
  font-family: system-ui, -apple-system, sans-serif;
  -webkit-font-smoothing: antialiased;
}

a {
  color: inherit;
  text-decoration: none;
}
"#,
    )?;

    eprintln!("  {} {}", green_bold("✓"), green_bold("Project created"));
    eprintln!();
    eprintln!("  {}", dim("Get started:"));
    eprintln!();
    eprintln!("    {} {}", bold("cd"), bold(&name));
    eprintln!("    {} {}", bold("npm install"), dim(""));
    eprintln!("    {} {}", bold("rex dev"), dim(""));
    eprintln!();

    Ok(())
}

fn cmd_lint(root: PathBuf, fix: bool, extra_args: Vec<String>) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;

    // Find oxlint binary
    let oxlint = find_oxlint(&root).ok_or_else(|| {
        anyhow::anyhow!(
            "oxlint not found. Install it:\n\n  \
             npm install -D oxlint\n  \
             # or globally: npm install -g oxlint"
        )
    })?;

    // Write default config if none exists
    let config_path = root.join(".oxlintrc.json");
    if !config_path.exists() {
        eprintln!(
            "  {} {}",
            dim("Creating"),
            dim(".oxlintrc.json with Rex defaults")
        );
        std::fs::write(&config_path, default_oxlintrc())?;
    }

    let pages_dir = root.join("pages");
    let lint_dir = if pages_dir.is_dir() {
        pages_dir
    } else {
        root.clone()
    };

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex lint"), dim("(oxlint)"));
    eprintln!();

    let mut cmd = Command::new(&oxlint);
    cmd.current_dir(&root);
    cmd.arg(lint_dir);
    cmd.arg("--config").arg(&config_path);

    if fix {
        cmd.arg("--fix");
    }

    for arg in &extra_args {
        cmd.arg(arg);
    }

    let status = cmd.status()?;

    if status.success() {
        eprintln!();
        eprintln!("  {} {}", green_bold("✓"), green_bold("No lint errors"));
        eprintln!();
    }

    std::process::exit(status.code().unwrap_or(1));
}

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

fn cmd_fmt(root: PathBuf, check: bool) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let files = discover_source_files(&root);

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

        let formatted = match format_source(&source, path) {
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

fn find_oxlint(root: &std::path::Path) -> Option<PathBuf> {
    // 1. Local node_modules/.bin/oxlint
    let local = root.join("node_modules/.bin/oxlint");
    if local.exists() {
        return Some(local);
    }

    // 2. oxlint in PATH
    if Command::new("oxlint")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return Some(PathBuf::from("oxlint"));
    }

    None
}

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
    "import/no-cycle": "warn"
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

async fn cmd_start(root: PathBuf, port: u16) -> Result<()> {
    let start = std::time::Instant::now();

    print_mascot_header(env!("CARGO_PKG_VERSION"), "(production)");

    let rex = rex_server::Rex::from_build(rex_server::RexOptions {
        root,
        dev: false,
        port,
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

async fn hmr_client_handler() -> impl axum::response::IntoResponse {
    let js = include_str!(concat!(env!("OUT_DIR"), "/hmr_client.js"));
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}

fn format_source(source: &str, path: &std::path::Path) -> Result<String> {
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
    let options = oxc_formatter::FormatOptions {
        quote_style: oxc_formatter::QuoteStyle::Single,
        ..Default::default()
    };
    Ok(oxc_formatter::Formatter::new(&allocator, options).build(&parsed.program))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_format_source_single_quotes() {
        let input = "const x = \"hello\";\n";
        let result = format_source(input, Path::new("test.ts")).unwrap();
        assert!(
            result.contains("'hello'"),
            "expected single quotes, got: {result}"
        );
    }

    #[test]
    fn test_format_source_semicolons() {
        let input = "const x = 1\n";
        let result = format_source(input, Path::new("test.ts")).unwrap();
        assert!(
            result.contains("const x = 1;"),
            "expected semicolons, got: {result}"
        );
    }

    #[test]
    fn test_format_source_tsx() {
        let input = "export default function App() { return <div>hi</div>; }\n";
        let result = format_source(input, Path::new("test.tsx")).unwrap();
        assert!(result.contains("<div>"), "expected JSX preserved: {result}");
    }

    #[test]
    fn test_format_source_idempotent() {
        let input = "const x = 'hello';\n";
        let first = format_source(input, Path::new("test.ts")).unwrap();
        let second = format_source(&first, Path::new("test.ts")).unwrap();
        assert_eq!(first, second, "formatting should be idempotent");
    }

    #[test]
    fn test_format_source_parse_error() {
        let input = "const = ;;\n";
        let result = format_source(input, Path::new("test.ts"));
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

        // Write already-formatted content
        let formatted = format_source("const x = \"hello\";\n", Path::new("t.ts")).unwrap();
        std::fs::write(pages.join("index.ts"), &formatted).unwrap();

        // Check mode should pass (return Ok)
        cmd_fmt(tmp.path().to_path_buf(), true).unwrap();
    }

    #[test]
    fn test_cmd_fmt_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No source files — should succeed silently
        cmd_fmt(tmp.path().to_path_buf(), false).unwrap();
    }

    #[test]
    fn test_cmd_fmt_skips_parse_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let pages = tmp.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("broken.ts"), "const = ;;\n").unwrap();
        std::fs::write(pages.join("good.ts"), "const x = \"hello\"\n").unwrap();

        // Should succeed despite one file having parse errors
        cmd_fmt(tmp.path().to_path_buf(), false).unwrap();

        // Broken file should be unchanged
        let broken = std::fs::read_to_string(pages.join("broken.ts")).unwrap();
        assert_eq!(broken, "const = ;;\n");

        // Good file should be formatted
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
}
