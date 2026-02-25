use anyhow::Result;
use clap::{Parser, Subcommand};
use rex_build::{build_bundles, AssetManifest};
use rex_core::RexConfig;
use rex_router::{scan_pages, RouteTrie};
use rex_server::RexServer;
use rex_v8::{init_v8, IsolatePool};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

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
    }
}

async fn cmd_dev(root: PathBuf, port: u16) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root).with_dev(true).with_port(port);
    config.validate()?;

    info!("Rex dev server starting...");

    // Scan routes
    let scan = scan_pages(&config.pages_dir)?;
    info!(
        routes = scan.routes.len(),
        has_app = scan.app.is_some(),
        has_404 = scan.not_found.is_some(),
        has_error = scan.error.is_some(),
        "Routes scanned"
    );

    // Build
    let build_result = build_bundles(&config, &scan).await?;
    info!(build_id = %build_result.build_id, "Build complete");

    // Initialize V8
    init_v8();

    // Load self-contained server bundle (includes React, polyfills, pages, SSR runtime)
    let server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)?;

    // Create isolate pool
    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(4); // Cap at 4 for dev

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
    );

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
                    &rebuild_state.isolate_pool,
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

    info!("Ready on http://localhost:{port}");
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}

async fn cmd_build(root: PathBuf) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root);
    config.validate()?;

    info!("Building...");

    let scan = scan_pages(&config.pages_dir)?;
    info!(routes = scan.routes.len(), "Routes scanned");

    let build_result = build_bundles(&config, &scan).await?;
    info!(build_id = %build_result.build_id, "Build complete");

    info!(
        server = %build_result.server_bundle_path.display(),
        manifest = %config.manifest_path().display(),
        "Output written"
    );

    Ok(())
}

async fn cmd_start(root: PathBuf, port: u16) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root).with_port(port);

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
    );

    info!("Rex production server starting on http://localhost:{port}");
    server.serve().await
}

async fn hmr_client_handler() -> impl axum::response::IntoResponse {
    let js = include_str!("../../../runtime/hmr_client.js");
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}
