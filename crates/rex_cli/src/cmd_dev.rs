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

    if !tui_enabled {
        print_mascot_header(env!("CARGO_PKG_VERSION"), "");
    }

    let esm_mode = std::env::var("REX_ESM_DEV").is_ok();

    let opts = rex_server::RexOptions {
        root: root.clone(),
        dev: true,
        port,
        host,
    };

    // Initialize Rex — ESM mode uses unbundled V8, standard mode uses rolldown
    let (rex, esm_state) = if esm_mode {
        let (rex, state) = rex_server::Rex::new_esm(opts).await?;
        (rex, Some(state))
    } else {
        let rex = rex_server::Rex::new(opts).await?;
        (rex, None)
    };

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

    if !tui_enabled && esm_mode {
        eprintln!("  {} {}", dim("◇"), dim("ESM dev mode (unbundled)"));
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

    // Build server with HMR websocket route + dev middleware (ESM mode)
    let hmr_route = axum::routing::get({
        let hmr_clone = hmr.clone();
        move |ws: axum::extract::ws::WebSocketUpgrade| async move {
            ws.on_upgrade(move |socket| rex_dev::hmr::handle_hmr_socket(socket, hmr_clone))
        }
    });

    let mut extra_routes = axum::Router::new().route("/_rex/hmr", hmr_route).route(
        "/_rex/hmr-client.js",
        axum::routing::get(hmr_client_handler),
    );

    // In ESM mode, add dev middleware routes for serving transformed sources and deps
    if let Some(ref esm) = esm_state {
        let dev_mw = rex_dev::dev_middleware::DevMiddleware::new(
            esm.transform_cache.clone(),
            esm.client_deps.clone(),
            config.project_root.clone(),
            esm.page_entries.clone(),
        );
        extra_routes = extra_routes.merge(dev_mw.into_router());
    }

    let router = rex.router_with_extra(extra_routes);

    // Spawn async rebuild handler
    {
        let rebuild_config = config.clone();
        let rebuild_hmr = hmr.clone();
        let rebuild_state = state.clone();
        let mut last_scan = Some(scan.clone());

        // ESM mode: use fast path (OXC transform + V8 module invalidation)
        // Standard mode: full rolldown rebuild
        let esm_transform_cache = esm_state.as_ref().map(|e| e.transform_cache.clone());
        let esm_page_sources = esm_state.as_ref().map(|e| e.page_sources.clone());

        tokio::spawn(async move {
            while let Some(event) = rebuild_rx.recv().await {
                if let (Some(ref cache), Some(ref pages)) =
                    (&esm_transform_cache, &esm_page_sources)
                {
                    info!(path = %event.path.display(), "ESM rebuilding...");
                    match rex_dev::rebuild::handle_esm_file_event(
                        event,
                        &rebuild_config,
                        &rebuild_state,
                        &rebuild_hmr,
                        cache,
                        pages,
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(e) => {
                            tracing::error!("ESM rebuild failed: {e}");
                            rebuild_hmr.send_error(&e.to_string(), None);
                        }
                    }
                } else {
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
