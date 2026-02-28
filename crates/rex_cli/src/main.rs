use anyhow::Result;
use clap::{Parser, Subcommand};
use rex_build::{build_bundles, AssetManifest};
use rex_core::{ProjectConfig, RexConfig};
use rex_router::{scan_pages, RouteTrie};
use rex_server::RexServer;
use rex_v8::{init_v8, IsolatePool};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, info};

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

    /// Create a new Rex project
    Init {
        /// Project name (creates a new directory)
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rex=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Dev { port, root } => cmd_dev(root, port).await,
        Commands::Build { root } => cmd_build(root).await,
        Commands::Start { port, root } => cmd_start(root, port).await,
        Commands::Lint { root, fix, args } => cmd_lint(root, fix, args),
        Commands::Init { name } => cmd_init(name),
    }
}

async fn cmd_dev(root: PathBuf, port: u16) -> Result<()> {
    let start = std::time::Instant::now();
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root).with_dev(true).with_port(port);
    config.validate()?;

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex"), dim(env!("CARGO_PKG_VERSION")));
    eprintln!();

    // Scan routes
    debug!("Scanning routes...");
    let scan = scan_pages(&config.pages_dir)?;
    debug!(
        routes = scan.routes.len(),
        has_app = scan.app.is_some(),
        has_404 = scan.not_found.is_some(),
        has_error = scan.error.is_some(),
        "Routes scanned"
    );

    // Build
    debug!("Building bundles...");
    let build_result = build_bundles(&config, &scan).await?;
    debug!(build_id = %build_result.build_id, "Build complete");

    // Initialize V8
    debug!("Initializing V8...");
    init_v8();

    // Load self-contained server bundle (includes React, polyfills, pages, SSR runtime)
    let server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)?;

    // Create isolate pool
    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(4); // Cap at 4 for dev

    debug!(pool_size, "Creating V8 isolate pool");
    let pool = IsolatePool::new(
        pool_size,
        Arc::new(server_bundle),
    )?;

    // Build route tries
    let trie = RouteTrie::from_routes(&scan.routes);
    let api_trie = RouteTrie::from_routes(&scan.api_routes);

    // Create HMR broadcast
    let hmr = rex_dev::HmrBroadcast::new();

    // Start file watcher
    let event_rx = rex_dev::start_watcher(&config.pages_dir)?;

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

    let extra_routes = axum::Router::new()
        .route("/_rex/hmr", hmr_route)
        .route("/_rex/hmr-client.js", axum::routing::get(hmr_client_handler));

    // Load project config (redirects, rewrites, headers)
    let project_config = ProjectConfig::load(&config.project_root)?;

    let server = RexServer::with_error_pages(
        trie,
        api_trie,
        pool,
        build_result.manifest,
        build_result.build_id,
        config.client_build_dir(),
        port,
        true,
        scan.not_found.is_some(),
        scan.error.is_some(),
        scan.document.is_some(),
        project_config,
    )
    .await;

    let router = server.build_router_with_extra(extra_routes);
    let state = server.state();

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
    eprintln!("  {} {}", green_bold("✓ Ready in"), green_bold(&format_duration(elapsed)));
    eprintln!();
    eprintln!("  {} {}", dim("➜ Local:"), bold(&format!("http://localhost:{port}")));
    eprintln!();
    print_route_summary(&scan.routes, &scan.api_routes);
    eprintln!();

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

async fn cmd_build(root: PathBuf) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root);
    config.validate()?;

    eprintln!();
    eprintln!("  {} {}", magenta_bold("◆ rex"), dim(env!("CARGO_PKG_VERSION")));
    eprintln!();

    let start = std::time::Instant::now();
    debug!("Building for production...");

    let scan = scan_pages(&config.pages_dir)?;
    debug!(routes = scan.routes.len(), "Routes scanned");

    let build_result = build_bundles(&config, &scan).await?;
    let elapsed = start.elapsed();

    eprintln!("  {} {}", green_bold("✓ Built in"), green_bold(&format_duration(elapsed)));
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
                let name = path.file_name().unwrap().to_string_lossy().to_string();
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
        eprintln!("    {}  {}", dim(&format!("{:<38}", name)), dim(&format_size(*size)));
    }

    // Show shared chunks
    chunk_sizes.sort_by(|a, b| b.1.cmp(&a.1)); // largest first
    for (name, size) in &chunk_sizes {
        eprintln!("    {}  {}", dim(&format!("{:<38}", name)), dim(&format_size(*size)));
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
        eprintln!("  {} {}", dim("Creating"), dim(".oxlintrc.json with Rex defaults"));
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
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root).with_port(port);

    eprintln!();
    eprintln!("  {} {} {}", magenta_bold("◆ rex"), dim(env!("CARGO_PKG_VERSION")), dim("(production)"));
    eprintln!();

    // Load manifest
    let manifest = AssetManifest::load(&config.manifest_path())?;

    // Scan routes (for trie)
    let scan = scan_pages(&config.pages_dir)?;
    let trie = RouteTrie::from_routes(&scan.routes);
    let api_trie = RouteTrie::from_routes(&scan.api_routes);

    // Initialize V8
    init_v8();

    let server_bundle = std::fs::read_to_string(config.server_bundle_path())?;

    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let pool = IsolatePool::new(
        pool_size,
        Arc::new(server_bundle),
    )?;

    // Load project config (redirects, rewrites, headers)
    let project_config = ProjectConfig::load(&config.project_root)?;

    let server = RexServer::with_error_pages(
        trie,
        api_trie,
        pool,
        manifest.clone(),
        manifest.build_id.clone(),
        config.client_build_dir(),
        port,
        false,
        scan.not_found.is_some(),
        scan.error.is_some(),
        scan.document.is_some(),
        project_config,
    )
    .await;

    let elapsed = start.elapsed();
    eprintln!("  {} {}", green_bold("✓ Ready in"), green_bold(&format_duration(elapsed)));
    eprintln!();
    eprintln!("  {} {}", dim("➜ Local:"), bold(&format!("http://localhost:{port}")));
    eprintln!();
    print_route_summary(&scan.routes, &scan.api_routes);
    eprintln!();

    server.serve().await
}

async fn hmr_client_handler() -> impl axum::response::IntoResponse {
    let js = include_str!("../../../runtime/hmr_client.js");
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}
