use anyhow::Result;
use clap::{Parser, Subcommand};
use rex_build::{build_bundles, AssetManifest};
use rex_core::RexConfig;
use rex_router::{scan_pages, RouteTrie};
use rex_server::RexServer;
use rex_v8::{init_v8, IsolatePool};
use std::collections::BTreeMap;
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
    let build_result = build_bundles(&config, &scan)?;
    info!(build_id = %build_result.build_id, "Build complete");

    // Load environment variables from .env files
    let env_vars = load_env_vars(&config);

    // Initialize V8
    init_v8();

    // Load React runtime (minimal CJS shim for V8)
    let react_runtime = load_react_runtime(&config, &env_vars)?;

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
        pool,
        build_result.manifest,
        build_result.build_id,
        config.client_build_dir(),
        port,
        true,
        scan.not_found.is_some(),
        scan.error.is_some(),
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

    // Load environment variables from .env files
    let env_vars = load_env_vars(&config);

    // Initialize V8
    init_v8();

    let react_runtime = load_react_runtime(&config, &env_vars)?;
    let server_bundle = std::fs::read_to_string(config.server_bundle_path())?;

    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let pool = IsolatePool::new(
        pool_size,
        Arc::new(react_runtime),
        Arc::new(server_bundle),
    )?;

    let server = RexServer::with_error_pages(
        trie,
        pool,
        manifest.clone(),
        manifest.build_id.clone(),
        config.client_build_dir(),
        port,
        false,
        scan.not_found.is_some(),
        scan.error.is_some(),
    );

    info!("Rex production server starting on http://localhost:{port}");
    server.serve().await
}

/// Load environment variables from .env files following Next.js priority order.
/// Later files take precedence over earlier ones; explicit env vars override all.
fn load_env_vars(config: &RexConfig) -> BTreeMap<String, String> {
    let root = &config.project_root;
    let mode = if config.dev { "development" } else { "production" };

    // Next.js load order (lowest to highest priority):
    // .env, .env.local, .env.{mode}, .env.{mode}.local
    let files = [
        root.join(".env"),
        root.join(".env.local"),
        root.join(format!(".env.{mode}")),
        root.join(format!(".env.{mode}.local")),
    ];

    let mut vars = BTreeMap::new();

    for file in &files {
        if file.exists() {
            match dotenvy::from_path_iter(file) {
                Ok(iter) => {
                    for item in iter {
                        if let Ok((key, value)) = item {
                            vars.insert(key, value);
                        }
                    }
                    info!(file = %file.display(), "Loaded env file");
                }
                Err(e) => {
                    tracing::warn!(file = %file.display(), error = %e, "Failed to load env file");
                }
            }
        }
    }

    // Always set NODE_ENV
    vars.entry("NODE_ENV".to_string())
        .or_insert_with(|| mode.to_string());

    vars
}

/// Generate a JS object literal for process.env from loaded environment variables.
fn env_vars_to_js(vars: &BTreeMap<String, String>) -> String {
    let mut js = String::from("{ ");
    for (i, (key, value)) in vars.iter().enumerate() {
        if i > 0 {
            js.push_str(", ");
        }
        // Escape key (identifiers) and value (strings)
        let escaped_value = value.replace('\\', "\\\\").replace('\'', "\\'");
        js.push_str(&format!("'{}': '{}'", key, escaped_value));
    }
    js.push_str(" }");
    js
}

/// Rex Head component runtime — loaded into V8 alongside React
const REX_HEAD_RUNTIME: &str = include_str!("../../../runtime/head.js");

/// Load React runtime for V8 SSR from node_modules.
/// Supports both CJS (React 19+) and UMD (React 18) builds.
fn load_react_runtime(config: &RexConfig, env_vars: &BTreeMap<String, String>) -> Result<String> {
    let nm = config.node_modules_dir();
    let env_js = env_vars_to_js(env_vars);

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

        let mut runtime = format!(
            r#"// React Runtime for V8 SSR (CJS)
if (typeof process === 'undefined') {{
    globalThis.process = {{ env: {env_js} }};
}}

// Polyfill Web APIs that React expects but V8 doesn't provide.
// Use Promise.resolve().then() to defer callbacks to the microtask queue
// instead of calling them synchronously. V8's perform_microtask_checkpoint()
// will drain these at the right time.
if (typeof MessageChannel === 'undefined') {{
    globalThis.MessageChannel = function() {{
        var channel = this;
        this.port1 = {{ onmessage: null }};
        this.port2 = {{
            postMessage: function() {{
                if (channel.port1.onmessage) {{
                    var cb = channel.port1.onmessage;
                    Promise.resolve().then(function() {{ cb({{ data: undefined }}); }});
                }}
            }}
        }};
    }};
}}
if (typeof setTimeout === 'undefined') {{
    var __timerIdCounter = 1;
    var __pendingTimers = {{}};
    globalThis.setTimeout = function(fn) {{
        var id = __timerIdCounter++;
        __pendingTimers[id] = true;
        Promise.resolve().then(function() {{
            if (__pendingTimers[id]) {{
                delete __pendingTimers[id];
                fn();
            }}
        }});
        return id;
    }};
    globalThis.clearTimeout = function(id) {{
        delete __pendingTimers[id];
    }};
}}
if (typeof queueMicrotask === 'undefined') {{
    globalThis.queueMicrotask = function(fn) {{ Promise.resolve().then(fn); }};
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

        runtime.push_str(REX_HEAD_RUNTIME);
        return Ok(runtime);
    }

    // Try UMD paths (React 18)
    let react_umd = nm.join("react/umd/react.production.min.js");
    let react_dom_server_umd = nm.join("react-dom/umd/react-dom-server.browser.production.min.js");

    if react_umd.exists() && react_dom_server_umd.exists() {
        let react_js = std::fs::read_to_string(&react_umd)?;
        let react_dom_js = std::fs::read_to_string(&react_dom_server_umd)?;

        let mut runtime = format!(
            r#"// React Runtime for V8 SSR (UMD)
if (typeof process === 'undefined') {{
    globalThis.process = {{ env: {env_js} }};
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

        runtime.push_str(REX_HEAD_RUNTIME);
        return Ok(runtime);
    }

    // Fallback: provide a stub that will error clearly
    info!("React not found in node_modules, using stub. Run `npm install react react-dom` in your project.");
    let mut stub = format!(r#"
// Stub React runtime - install react and react-dom for real SSR
if (typeof process === 'undefined') {{
    globalThis.process = {{ env: {env_js} }};
}}
globalThis.__React = {{
    createElement: function(type, props) {{
        var children = Array.prototype.slice.call(arguments, 2);
        return {{ type: type, props: props || {{}}, children: children, $$typeof: Symbol.for('react.element') }};
    }},
    Fragment: Symbol.for('react.fragment'),
}};
globalThis.__ReactDOMServer = {{
    renderToString: function(element) {{
        if (!element) return '';
        if (typeof element === 'string') return element;
        if (typeof element.type === 'function') {{
            var result = element.type(element.props);
            return globalThis.__ReactDOMServer.renderToString(result);
        }}
        if (typeof element.type === 'string') {{
            var tag = element.type;
            var html = '<' + tag;
            if (element.props) {{
                Object.keys(element.props).forEach(function(key) {{
                    if (key === 'children' || key === 'dangerouslySetInnerHTML') return;
                    if (key === 'className') {{
                        html += ' class="' + element.props[key] + '"';
                    }} else if (typeof element.props[key] === 'string') {{
                        html += ' ' + key + '="' + element.props[key] + '"';
                    }}
                }});
            }}
            html += '>';
            var children = element.props && element.props.children;
            if (children) {{
                if (Array.isArray(children)) {{
                    children.forEach(function(child) {{
                        html += globalThis.__ReactDOMServer.renderToString(child);
                    }});
                }} else {{
                    html += globalThis.__ReactDOMServer.renderToString(children);
                }}
            }}
            if (element.children && element.children.length > 0) {{
                element.children.forEach(function(child) {{
                    html += globalThis.__ReactDOMServer.renderToString(child);
                }});
            }}
            html += '</' + tag + '>';
            return html;
        }}
        return '';
    }},
}};
globalThis.React = globalThis.__React;
globalThis.ReactDOMServer = globalThis.__ReactDOMServer;
"#);
    stub.push_str(REX_HEAD_RUNTIME);
    Ok(stub)
}

async fn hmr_client_handler() -> impl axum::response::IntoResponse {
    let js = include_str!("../../../runtime/hmr_client.js");
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js,
    )
}
