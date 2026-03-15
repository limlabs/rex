use anyhow::{Context, Result};
use std::net::IpAddr;
use std::path::PathBuf;
use tracing::info;

use crate::display::*;
use crate::tui;
use crate::tui::log_layer::LogBuffer;

pub(crate) async fn cmd_dev(
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

    // Register this instance for MCP discovery
    if let Err(e) = rex_core::instance::register_instance(port, &host.to_string(), &root) {
        tracing::warn!("Failed to register dev instance: {e}");
    }

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

    // Create error buffer and HMR broadcast
    let error_buffer = rex_core::ErrorBuffer::new(64);
    let hmr = rex_dev::HmrBroadcast::with_error_buffer(error_buffer.clone());

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

    let extra_routes = axum::Router::new()
        .route("/_rex/hmr", hmr_route)
        .route(
            "/_rex/hmr-client.js",
            axum::routing::get(hmr_client_handler),
        )
        .route(
            "/_rex/dev/status",
            axum::routing::get(rex_server::dev_introspection::status_handler),
        )
        .route(
            "/_rex/dev/routes",
            axum::routing::get(rex_server::dev_introspection::routes_handler),
        )
        .route(
            "/_rex/dev/errors",
            axum::routing::get(rex_server::dev_introspection::errors_handler),
        )
        .layer(axum::Extension(error_buffer));

    let router = rex.router_with_extra(extra_routes);

    // Spawn async rebuild handler: rebuild bundles + reload V8 isolates on file changes
    {
        let rebuild_config = config.clone();
        let rebuild_hmr = hmr.clone();
        let rebuild_state = state.clone();
        let mut last_scan = Some(scan.clone());
        tokio::spawn(async move {
            while let Some(event) = rebuild_rx.recv().await {
                info!(path = %event.path.display(), "Rebuilding...");
                match rex_dev::rebuild::handle_file_event(
                    event,
                    &rebuild_config,
                    &rebuild_state,
                    &rebuild_hmr,
                    &mut last_scan,
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

        // The TUI has exited — clean up instance registration and force-quit.
        // Background tasks (file watcher, rebuild handler, HTTP server) have no
        // shutdown signal and would block the tokio runtime's graceful shutdown.
        rex_core::instance::unregister_instance();
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

pub(crate) fn cmd_typecheck(root: PathBuf, extra_args: Vec<String>) -> Result<()> {
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

    let mut cmd = std::process::Command::new(&tsc);
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

async fn hmr_client_handler() -> impl axum::response::IntoResponse {
    let js = include_str!(concat!(env!("OUT_DIR"), "/hmr_client.js"));
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}
