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
        "Routes scanned"
    );

    // Build
    let build_result = build_bundles(&config, &scan)?;
    info!(build_id = %build_result.build_id, "Build complete");

    // Initialize V8
    init_v8();

    // Load React runtime (minimal CJS shim for V8)
    let react_runtime = load_react_runtime(&config)?;

    // Load server bundle
    let server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)?;

    // Create isolate pool
    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(4); // Cap at 4 for dev

    let pool = IsolatePool::new(
        pool_size,
        Arc::new(react_runtime),
        Arc::new(server_bundle),
    )?;

    // Build route trie
    let trie = RouteTrie::from_routes(&scan.routes);

    // Create HMR broadcast
    let hmr = rex_dev::HmrBroadcast::new();

    // Start file watcher
    let event_rx = rex_dev::start_watcher(&config.pages_dir)?;

    // Spawn rebuild handler
    let _config_clone = config.clone();
    let hmr_clone = hmr.clone();
    // We can't easily share the pool for reload in this prototype,
    // so file changes will trigger a full reload message
    tokio::task::spawn_blocking(move || {
        while let Ok(event) = event_rx.recv() {
            info!(path = %event.path.display(), "File changed, signaling reload");
            hmr_clone.send_full_reload();
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

    let server = RexServer::new(
        trie,
        pool,
        build_result.manifest,
        build_result.build_id,
        config.client_build_dir(),
        port,
        true,
    );

    info!("Ready on http://localhost:{port}");
    let router = server.build_router_with_extra(extra_routes);
    // Keep an Arc<AppState> reference alive across the .await points below.
    // Without this, the async transform may drop `server` (and its Arc) before
    // the serve future runs, causing the IsolatePool senders to drop and the
    // V8 isolate threads to exit immediately.
    let _keep = server.state();
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

    let build_result = build_bundles(&config, &scan)?;
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

    // Initialize V8
    init_v8();

    let react_runtime = load_react_runtime(&config)?;
    let server_bundle = std::fs::read_to_string(config.server_bundle_path())?;

    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let pool = IsolatePool::new(
        pool_size,
        Arc::new(react_runtime),
        Arc::new(server_bundle),
    )?;

    let server = RexServer::new(
        trie,
        pool,
        manifest.clone(),
        manifest.build_id.clone(),
        config.client_build_dir(),
        port,
        false,
    );

    info!("Rex production server starting on http://localhost:{port}");
    server.serve().await
}

/// Load React runtime for V8 SSR from node_modules.
/// Supports both CJS (React 19+) and UMD (React 18) builds.
fn load_react_runtime(config: &RexConfig) -> Result<String> {
    let nm = config.node_modules_dir();

    // Try CJS paths first (React 19+)
    let react_cjs = nm.join("react/cjs/react.production.js");
    // React 19 moved renderToString to the "legacy" server module;
    // the non-legacy module only exposes renderToReadableStream.
    let react_dom_server_cjs = nm.join("react-dom/cjs/react-dom-server-legacy.browser.production.js");

    if react_cjs.exists() && react_dom_server_cjs.exists() {
        let react_js = std::fs::read_to_string(&react_cjs)?;
        let react_dom_server_js = std::fs::read_to_string(&react_dom_server_cjs)?;

        // react-dom-server requires 'react' and 'react-dom', so load react-dom base too
        let react_dom_cjs = nm.join("react-dom/cjs/react-dom.production.js");
        let react_dom_js = if react_dom_cjs.exists() {
            std::fs::read_to_string(&react_dom_cjs)?
        } else {
            String::new()
        };

        let runtime = format!(
            r#"// React Runtime for V8 SSR (CJS)
if (typeof process === 'undefined') {{
    globalThis.process = {{ env: {{ NODE_ENV: 'production' }} }};
}}

// Polyfill Web APIs that React expects but V8 doesn't provide
if (typeof MessageChannel === 'undefined') {{
    globalThis.MessageChannel = function() {{
        this.port1 = {{}};
        this.port2 = {{ postMessage: function() {{}} }};
    }};
}}
if (typeof setTimeout === 'undefined') {{
    globalThis.setTimeout = function(fn) {{ fn(); return 0; }};
    globalThis.clearTimeout = function() {{}};
}}
if (typeof queueMicrotask === 'undefined') {{
    globalThis.queueMicrotask = function(fn) {{ fn(); }};
}}
if (typeof performance === 'undefined') {{
    globalThis.performance = {{ now: function() {{ return Date.now(); }} }};
}}
if (typeof TextEncoder === 'undefined') {{
    globalThis.TextEncoder = function() {{}};
    globalThis.TextEncoder.prototype.encode = function(s) {{
        var arr = [];
        for (var i = 0; i < s.length; i++) arr.push(s.charCodeAt(i));
        return new Uint8Array(arr);
    }};
}}
if (typeof TextDecoder === 'undefined') {{
    globalThis.TextDecoder = function() {{}};
    globalThis.TextDecoder.prototype.decode = function(arr) {{
        var s = '';
        for (var i = 0; i < arr.length; i++) s += String.fromCharCode(arr[i]);
        return s;
    }};
}}

var __modules = {{}};

// React CJS
(function() {{
    var exports = {{}};
    var module = {{ exports: exports }};
{react_js}
    __modules['react'] = module.exports;
}})();
globalThis.__React = __modules['react'];
globalThis.React = __modules['react'];

// ReactDOM CJS
(function() {{
    var exports = {{}};
    var module = {{ exports: exports }};
    var require = function(name) {{ return __modules[name]; }};
{react_dom_js}
    __modules['react-dom'] = module.exports;
}})();

// ReactDOMServer CJS
(function() {{
    var exports = {{}};
    var module = {{ exports: exports }};
    var require = function(name) {{ return __modules[name]; }};
{react_dom_server_js}
    __modules['react-dom/server'] = module.exports;
}})();
globalThis.__ReactDOMServer = __modules['react-dom/server'];
globalThis.ReactDOMServer = __modules['react-dom/server'];
"#
        );

        return Ok(runtime);
    }

    // Try UMD paths (React 18)
    let react_umd = nm.join("react/umd/react.production.min.js");
    let react_dom_server_umd = nm.join("react-dom/umd/react-dom-server.browser.production.min.js");

    if react_umd.exists() && react_dom_server_umd.exists() {
        let react_js = std::fs::read_to_string(&react_umd)?;
        let react_dom_js = std::fs::read_to_string(&react_dom_server_umd)?;

        let runtime = format!(
            r#"// React Runtime for V8 SSR (UMD)
if (typeof process === 'undefined') {{
    globalThis.process = {{ env: {{ NODE_ENV: 'production' }} }};
}}

(function() {{
{react_js}
}})();
globalThis.__React = globalThis.React;

(function() {{
{react_dom_js}
}})();
globalThis.__ReactDOMServer = globalThis.ReactDOMServer;
"#
        );

        return Ok(runtime);
    }

    // Fallback: provide a stub that will error clearly
    info!("React not found in node_modules, using stub. Run `npm install react react-dom` in your project.");
    Ok(r#"
// Stub React runtime - install react and react-dom for real SSR
globalThis.__React = {
    createElement: function(type, props) {
        var children = Array.prototype.slice.call(arguments, 2);
        return { type: type, props: props || {}, children: children, $$typeof: Symbol.for('react.element') };
    },
    Fragment: Symbol.for('react.fragment'),
};
globalThis.__ReactDOMServer = {
    renderToString: function(element) {
        if (!element) return '';
        if (typeof element === 'string') return element;
        if (typeof element.type === 'function') {
            var result = element.type(element.props);
            return globalThis.__ReactDOMServer.renderToString(result);
        }
        if (typeof element.type === 'string') {
            var tag = element.type;
            var html = '<' + tag;
            if (element.props) {
                Object.keys(element.props).forEach(function(key) {
                    if (key === 'children' || key === 'dangerouslySetInnerHTML') return;
                    if (key === 'className') {
                        html += ' class="' + element.props[key] + '"';
                    } else if (typeof element.props[key] === 'string') {
                        html += ' ' + key + '="' + element.props[key] + '"';
                    }
                });
            }
            html += '>';
            var children = element.props && element.props.children;
            if (children) {
                if (Array.isArray(children)) {
                    children.forEach(function(child) {
                        html += globalThis.__ReactDOMServer.renderToString(child);
                    });
                } else {
                    html += globalThis.__ReactDOMServer.renderToString(children);
                }
            }
            if (element.children && element.children.length > 0) {
                element.children.forEach(function(child) {
                    html += globalThis.__ReactDOMServer.renderToString(child);
                });
            }
            html += '</' + tag + '>';
            return html;
        }
        return '';
    },
};
globalThis.React = globalThis.__React;
globalThis.ReactDOMServer = globalThis.__ReactDOMServer;
"#.to_string())
}

async fn hmr_client_handler() -> impl axum::response::IntoResponse {
    let js = include_str!("../../../runtime/hmr_client.js");
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}
