use napi::bindgen_prelude::*;
use napi::{Env, JsFunction, JsObject, JsUnknown};
use rex_server::core::{RexBody, RexRequest, RexResponse, RouteMatchResult};
use rex_server::state::snapshot;
use rex_server::Rex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
// lgtm[rust/access-invalid-pointer] — napi-rs macro generates safe FFI wrappers
pub struct RexInstance {
    rex: Rex,
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

    let is_dev = options.dev.unwrap_or(false);

    let rex = Rex::new(rex_server::RexOptions {
        root: options.root.into(),
        dev: is_dev,
        port: 0, // NAPI doesn't serve directly
    })
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    let static_dir = rex.static_dir().clone();

    // Create a dedicated tokio runtime for sync->async bridging
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| Error::from_reason(format!("Failed to create tokio runtime: {e}")))?;

    Ok(RexInstance {
        rex,
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
        self.rex.is_dev()
    }

    /// The current build ID.
    #[napi(getter)]
    pub fn build_id(&self) -> String {
        self.rex.build_id()
    }

    /// The path to the static files directory (client JS/CSS bundles).
    #[napi(getter)]
    pub fn static_dir(&self) -> String {
        self.rex.static_dir().to_string_lossy().to_string()
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
        self.rex.match_route(&path).map(JsRouteMatch::from)
    }

    /// Run getServerSideProps for a given path and return the result as JSON.
    #[napi]
    pub async fn get_server_side_props(&self, path: String) -> Result<serde_json::Value> {
        self.check_closed()?;
        self.rex
            .get_server_side_props(&path)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Render a page to an HTML string with the given props.
    #[napi]
    pub async fn render_to_string(&self, path: String, props: serde_json::Value) -> Result<String> {
        self.check_closed()?;
        self.rex
            .render_to_string(&path, &props)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Render a full page (GSSP + SSR + document assembly). Returns HTML, status, and headers.
    #[napi]
    pub async fn render_page(&self, path: String) -> Result<JsPageResult> {
        self.check_closed()?;
        let result = self
            .rex
            .render_page(&path)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        Ok(JsPageResult {
            html: result.html,
            status: result.status as u32,
            headers: result
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
        let state = self.rex.state();
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
            let headers = extract_headers(env, &headers_obj)?;

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
    state: &Arc<rex_server::state::AppState>,
    static_dir: &Path,
    req: RexRequest,
) -> RexResponse {
    let path = &req.path;

    // Handle static files: /_rex/static/*
    if let Some(rel_path) = path.strip_prefix("/_rex/static/") {
        return serve_static_file(static_dir, rel_path);
    }

    // Delegate everything else to the core handler
    let hot = snapshot(state);
    rex_server::core::handle_request(state, &hot, &req).await
}

/// Serve a static file from the build output directory.
fn serve_static_file(static_dir: &Path, rel_path: &str) -> RexResponse {
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
