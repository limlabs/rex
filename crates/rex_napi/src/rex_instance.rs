use napi::bindgen_prelude::*;
use napi::{Env, JsFunction, JsObject, JsUnknown};
use rex_core::{ProjectConfig, RexConfig};
use rex_router::{scan_pages, RouteTrie};
use rex_server::core::{self, RexBody, RexRequest, RexResponse, RouteMatchResult};
use rex_server::handlers::{snapshot, AppState, HotState};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tracing::debug;

/// Options for creating a Rex instance.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct RexOptions {
    /// Path to the project root directory (containing pages/).
    pub root: String,
    /// Whether to run in dev mode (enables HMR, error overlays).
    pub dev: Option<bool>,
}

/// Result from matching a route.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct JsRouteMatch {
    /// The route pattern, e.g. "/blog/:slug"
    pub pattern: String,
    /// The module name, e.g. "blog/[slug]"
    pub module_name: String,
    /// Matched params, e.g. { slug: "hello" }
    pub params: HashMap<String, String>,
}

impl From<RouteMatchResult> for JsRouteMatch {
    fn from(m: RouteMatchResult) -> Self {
        Self {
            pattern: m.pattern,
            module_name: m.module_name,
            params: m.params,
        }
    }
}

/// Result from renderPage.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct JsPageResult {
    /// Full HTML document.
    pub html: String,
    /// HTTP status code.
    pub status: u32,
    /// Response headers.
    pub headers: Vec<JsHeaderPair>,
}

/// A header key-value pair.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct JsHeaderPair {
    pub key: String,
    pub value: String,
}

/// A Rex application instance backed by Rust's V8 isolate pool and SSR engine.
///
/// Created via `createRex()`. Handles route matching, server-side rendering,
/// and request handling for Rex applications.
#[napi]
pub struct RexInstance {
    state: Arc<AppState>,
    _config: RexConfig,
    static_dir: PathBuf,
    closed: AtomicBool,
    // Keep the tokio runtime alive for async operations dispatched from sync NAPI calls
    _rt: tokio::runtime::Runtime,
}

/// Create a new Rex application instance.
///
/// Scans the pages directory, builds bundles, initializes the V8 isolate pool,
/// and returns a ready-to-use RexInstance.
///
/// ```js
/// const rex = await createRex({ root: './my-app' })
/// ```
#[napi]
pub async fn create_rex(options: RexOptions) -> Result<RexInstance> {
    // Initialize tracing (only once, ignore errors if already initialized)
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    let root = std::fs::canonicalize(&options.root)
        .map_err(|e| Error::from_reason(format!("Invalid root path '{}': {e}", options.root)))?;

    let is_dev = options.dev.unwrap_or(false);
    let config = RexConfig::new(root.clone()).with_dev(is_dev);

    config
        .validate()
        .map_err(|e| Error::from_reason(e.to_string()))?;

    let project_config = ProjectConfig::load(&config.project_root)
        .map_err(|e| Error::from_reason(e.to_string()))?;

    // Scan pages
    debug!("Scanning routes...");
    let scan = scan_pages(&config.pages_dir)
        .map_err(|e| Error::from_reason(format!("Failed to scan pages: {e}")))?;

    // Build bundles
    debug!("Building bundles...");
    let build_result = rex_build::build_bundles(&config, &scan, &project_config)
        .await
        .map_err(|e| Error::from_reason(format!("Build failed: {e}")))?;

    // Initialize V8
    debug!("Initializing V8...");
    rex_v8::init_v8();

    let server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)
        .map_err(|e| Error::from_reason(format!("Failed to read server bundle: {e}")))?;

    let pool_size = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(if is_dev { 4 } else { 8 });

    debug!(pool_size, "Creating V8 isolate pool");
    let pool = rex_v8::IsolatePool::new(pool_size, Arc::new(server_bundle))
        .map_err(|e| Error::from_reason(format!("Failed to create V8 pool: {e}")))?;

    // Build route tries
    let trie = RouteTrie::from_routes(&scan.routes);
    let api_trie = RouteTrie::from_routes(&scan.api_routes);

    let build_id = build_result.build_id.clone();
    let manifest = build_result.manifest;
    let manifest_json = HotState::compute_manifest_json(&build_id, &manifest);

    // Compute document descriptor if custom _document exists
    let document_descriptor = if scan.document.is_some() {
        rex_server::handlers::compute_document_descriptor(&pool).await
    } else {
        None
    };

    let image_cache = rex_image::ImageCache::new(
        config.project_root.join(".rex").join("cache").join("images"),
    );

    let state = Arc::new(AppState {
        isolate_pool: pool,
        is_dev,
        project_root: config.project_root.clone(),
        image_cache,
        hot: RwLock::new(Arc::new(HotState {
            route_trie: trie,
            api_route_trie: api_trie,
            manifest,
            build_id,
            has_custom_404: scan.not_found.is_some(),
            has_custom_error: scan.error.is_some(),
            has_custom_document: scan.document.is_some(),
            project_config,
            manifest_json,
            document_descriptor,
        })),
    });

    let static_dir = config.client_build_dir();

    // Create a dedicated tokio runtime for sync->async bridging
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| Error::from_reason(format!("Failed to create tokio runtime: {e}")))?;

    Ok(RexInstance {
        state,
        _config: config,
        static_dir,
        closed: AtomicBool::new(false),
        _rt: rt,
    })
}

#[napi]
impl RexInstance {
    /// Whether this instance is running in dev mode.
    #[napi(getter)]
    pub fn is_dev(&self) -> bool {
        self.state.is_dev
    }

    /// The current build ID.
    #[napi(getter)]
    pub fn build_id(&self) -> String {
        let hot = snapshot(&self.state);
        hot.build_id.clone()
    }

    /// The path to the static files directory (client JS/CSS bundles).
    #[napi(getter)]
    pub fn static_dir(&self) -> String {
        self.static_dir.to_string_lossy().to_string()
    }

    /// Match a URL path against the route trie.
    ///
    /// Returns the matched route info with params, or null if no match.
    /// ```js
    /// const match = rex.matchRoute('/blog/hello')
    /// // { pattern: '/blog/:slug', moduleName: 'blog/[slug]', params: { slug: 'hello' } }
    /// ```
    #[napi]
    pub fn match_route(&self, path: String) -> Option<JsRouteMatch> {
        self.check_closed().ok()?;
        let hot = snapshot(&self.state);
        core::match_route(&hot, &path).map(JsRouteMatch::from)
    }

    /// Run getServerSideProps for a given path and return the result as JSON.
    #[napi]
    pub async fn get_server_side_props(&self, path: String) -> Result<serde_json::Value> {
        self.check_closed()?;
        let hot = snapshot(&self.state);

        let route_match = hot
            .route_trie
            .match_path(&path)
            .ok_or_else(|| Error::from_reason(format!("No route matches path: {path}")))?;

        let route_key = route_match.route.module_name();
        let params = route_match.params.clone();

        let strategy = hot
            .manifest
            .pages
            .get(&route_match.route.pattern)
            .map(|p| &p.data_strategy)
            .cloned()
            .unwrap_or_default();

        let result = match strategy {
            rex_core::DataStrategy::None => Ok(Ok(r#"{"props":{}}"#.to_string())),
            rex_core::DataStrategy::GetStaticProps => {
                let ctx_json = serde_json::json!({ "params": params }).to_string();
                self.state
                    .isolate_pool
                    .execute(move |iso| iso.get_static_props(&route_key, &ctx_json))
                    .await
            }
            rex_core::DataStrategy::GetServerSideProps => {
                let context = rex_core::ServerSidePropsContext {
                    params,
                    query: HashMap::new(),
                    resolved_url: path.clone(),
                    headers: HashMap::new(),
                    cookies: HashMap::new(),
                };
                let context_json = serde_json::to_string(&context).expect("JSON serialization");
                self.state
                    .isolate_pool
                    .execute(move |iso| iso.get_server_side_props(&route_key, &context_json))
                    .await
            }
        };

        let json_str = match result {
            Ok(Ok(json)) => json,
            Ok(Err(e)) => return Err(Error::from_reason(format!("GSSP error: {e}"))),
            Err(e) => return Err(Error::from_reason(format!("Pool error: {e}"))),
        };

        serde_json::from_str(&json_str)
            .map_err(|e| Error::from_reason(format!("Failed to parse GSSP result: {e}")))
    }

    /// Render a page to an HTML string with the given props.
    #[napi]
    pub async fn render_to_string(
        &self,
        path: String,
        props: serde_json::Value,
    ) -> Result<String> {
        self.check_closed()?;
        let hot = snapshot(&self.state);

        let route_match = hot
            .route_trie
            .match_path(&path)
            .ok_or_else(|| Error::from_reason(format!("No route matches path: {path}")))?;

        let route_key = route_match.route.module_name();
        let props_json = serde_json::to_string(&props).expect("JSON serialization");

        let result = self
            .state
            .isolate_pool
            .execute(move |iso| iso.render_page(&route_key, &props_json))
            .await;

        match result {
            Ok(Ok(r)) => Ok(r.body),
            Ok(Err(e)) => Err(Error::from_reason(format!("SSR render error: {e}"))),
            Err(e) => Err(Error::from_reason(format!("Pool error: {e}"))),
        }
    }

    /// Render a full page (GSSP + SSR + document assembly). Returns HTML, status, and headers.
    #[napi]
    pub async fn render_page(&self, path: String) -> Result<JsPageResult> {
        self.check_closed()?;
        let hot = snapshot(&self.state);
        let req = RexRequest {
            method: "GET".to_string(),
            path: path.clone(),
            query: None,
            headers: HashMap::new(),
            body: Vec::new(),
        };

        let resp = core::handle_page(&self.state, &hot, &req).await;

        Ok(JsPageResult {
            html: body_to_string(&resp.body),
            status: resp.status as u32,
            headers: resp
                .headers
                .into_iter()
                .map(|(k, v)| JsHeaderPair { key: k, value: v })
                .collect(),
        })
    }

    /// Get a request handler function compatible with the Web Fetch API.
    ///
    /// Returns a function `(Request) => Promise<Response>` that handles all Rex routes.
    /// Works with Bun.serve, Deno.serve, and Node.js 18+ (which all support Web Request/Response).
    ///
    /// ```js
    /// const handler = rex.getRequestHandler()
    /// Bun.serve({ fetch: handler })
    /// ```
    #[napi(ts_return_type = "(req: Request) => Promise<Response>")]
    pub fn get_request_handler(&self, env: Env) -> Result<JsFunction> {
        self.check_closed()?;
        let state = self.state.clone();
        let static_dir = self.static_dir.clone();

        env.create_function_from_closure("rexHandler", move |ctx| {
            let request = ctx.get::<JsObject>(0)?;
            let env = ctx.env;
            let state = state.clone();
            let static_dir = static_dir.clone();

            // Extract request info from the JS Request object
            let method: String = request.get_named_property("method")?;
            let url_str: String = request.get_named_property("url")?;

            // Parse the URL to get path and query
            let (path, query) = parse_url(&url_str);

            // Extract headers from the JS Request
            let headers_obj: JsObject = request.get_named_property("headers")?;
            let headers = extract_headers(&env, &headers_obj)?;

            // Build the RexRequest
            let rex_req = RexRequest {
                method,
                path: path.clone(),
                query,
                headers,
                body: Vec::new(), // TODO: body extraction for POST etc.
            };

            // Create a promise for the async response
            let (deferred, promise) = env.create_deferred()?;

            // Spawn the async work
            napi::bindgen_prelude::spawn(async move {
                let response = dispatch_request(&state, &static_dir, rex_req).await;
                deferred.resolve(move |env| response_to_js(&env, &response));
            });

            Ok(promise)
        })
    }

    /// Shut down the Rex instance, releasing V8 isolates and other resources.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        // The IsolatePool's Drop impl will handle graceful shutdown
        // when the Arc<AppState> reference count reaches 0.
        // We just mark ourselves as closed so future calls fail fast.
        Ok(())
    }

    fn check_closed(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::from_reason("RexInstance is closed"));
        }
        Ok(())
    }
}

/// Parse a URL string into (path, optional query string).
fn parse_url(url_str: &str) -> (String, Option<String>) {
    // Handle both full URLs (http://localhost:3000/path) and path-only (/path)
    if let Some(path_start) = url_str.find("://") {
        // Full URL - find the path after host
        let after_scheme = &url_str[path_start + 3..];
        let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
        let path_and_query = &after_scheme[path_start..];
        split_path_query(if path_and_query.is_empty() {
            "/"
        } else {
            path_and_query
        })
    } else {
        split_path_query(url_str)
    }
}

fn split_path_query(path_and_query: &str) -> (String, Option<String>) {
    if let Some(q_pos) = path_and_query.find('?') {
        let path = &path_and_query[..q_pos];
        let query = &path_and_query[q_pos + 1..];
        (
            if path.is_empty() {
                "/".to_string()
            } else {
                path.to_string()
            },
            Some(query.to_string()),
        )
    } else {
        (
            if path_and_query.is_empty() {
                "/".to_string()
            } else {
                path_and_query.to_string()
            },
            None,
        )
    }
}

/// Extract headers from a JS Headers object.
fn extract_headers(env: &Env, headers_obj: &JsObject) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();

    // Use headers.entries() iterator pattern
    // But simpler: try common headers we care about
    let common_headers = [
        "accept",
        "accept-encoding",
        "accept-language",
        "content-type",
        "content-length",
        "cookie",
        "host",
        "referer",
        "user-agent",
        "x-forwarded-for",
        "x-forwarded-proto",
        "authorization",
    ];

    // Try headers.get(name) method
    let get_fn: JsFunction = headers_obj.get_named_property("get")?;
    for name in &common_headers {
        let js_name = env.create_string(name)?;
        let result: JsUnknown = get_fn.call(Some(headers_obj), &[js_name])?;
        if result.get_type()? == napi::ValueType::String {
            let value: String = result.coerce_to_string()?.into_utf8()?.into_owned()?;
            headers.insert(name.to_string(), value);
        }
    }

    Ok(headers)
}

/// Dispatch a request through the Rex core handler, including static file serving.
async fn dispatch_request(
    state: &Arc<AppState>,
    static_dir: &PathBuf,
    req: RexRequest,
) -> RexResponse {
    let path = &req.path;

    // Handle static files: /_rex/static/*
    if let Some(rel_path) = path.strip_prefix("/_rex/static/") {
        return serve_static_file(static_dir, rel_path);
    }

    // Delegate everything else to the core handler
    let hot = snapshot(state);
    core::handle_request(state, &hot, &req).await
}

/// Serve a static file from the build output directory.
fn serve_static_file(static_dir: &PathBuf, rel_path: &str) -> RexResponse {
    let file_path = static_dir.join(rel_path);

    // Prevent path traversal
    if let (Ok(canonical), Ok(base)) = (file_path.canonicalize(), static_dir.canonicalize()) {
        if !canonical.starts_with(&base) {
            return RexResponse::text(400, "Invalid path".to_string());
        }
    }

    let data = match std::fs::read(&file_path) {
        Ok(d) => d,
        Err(_) => return RexResponse::not_found(),
    };

    let content_type = guess_content_type(rel_path);
    RexResponse {
        status: 200,
        headers: vec![
            ("content-type".to_string(), content_type.to_string()),
            (
                "cache-control".to_string(),
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        body: RexBody::Full(data),
    }
}

fn guess_content_type(path: &str) -> &'static str {
    if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".map") {
        "application/json"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    }
}

fn body_to_string(body: &RexBody) -> String {
    match body {
        RexBody::Full(bytes) => String::from_utf8_lossy(bytes).to_string(),
        RexBody::Empty => String::new(),
    }
}

/// Convert a RexResponse into a JS Response object.
fn response_to_js(env: &Env, resp: &RexResponse) -> Result<JsUnknown> {
    // Get the global Response constructor
    let global = env.get_global()?;
    let response_ctor: JsFunction = global.get_named_property("Response")?;

    // Create body
    let body = match &resp.body {
        RexBody::Full(bytes) => env.create_buffer_with_data(bytes.to_vec())?.into_unknown(),
        RexBody::Empty => env.get_null()?.into_unknown(),
    };

    // Create init object { status, headers }
    let mut init = env.create_object()?;
    init.set_named_property("status", env.create_int32(resp.status as i32)?)?;

    // Build headers as a plain object (Response constructor accepts this)
    let mut headers_obj = env.create_object()?;
    for (k, v) in &resp.headers {
        headers_obj.set_named_property(k.as_str(), env.create_string(v)?)?;
    }
    init.set_named_property("headers", headers_obj)?;

    // new Response(body, init)
    let response = response_ctor.new_instance(&[body, init.into_unknown()])?;
    Ok(response.into_unknown())
}
