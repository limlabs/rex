use crate::handlers;
use crate::state::{AppState, HotState};
use anyhow::Result;
use axum::handler::Handler;
use axum::response::IntoResponse;
use axum::routing::{any, get, post};
use axum::Router;
use rex_core::{AssetManifest, ProjectConfig};
use rex_router::RouteTrie;
use rex_v8::IsolatePool;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tracing::info;

/// Configuration for constructing a `RexServer`.
pub struct ServerConfig {
    pub route_trie: RouteTrie,
    pub api_route_trie: RouteTrie,
    pub isolate_pool: IsolatePool,
    pub manifest: AssetManifest,
    pub build_id: String,
    pub static_dir: PathBuf,
    pub project_root: PathBuf,
    pub port: u16,
    pub is_dev: bool,
    pub has_custom_404: bool,
    pub has_custom_error: bool,
    pub has_custom_document: bool,
    pub project_config: ProjectConfig,
    pub has_middleware: bool,
    pub middleware_matchers: Option<Vec<String>>,
    /// App page route trie for RSC app/ routes. None if no app/ directory.
    pub app_route_trie: Option<RouteTrie>,
    /// App API route trie for app/ route handlers (route.ts). None if no route.ts files.
    pub app_api_route_trie: Option<RouteTrie>,
    pub has_mcp_tools: bool,
    pub host: IpAddr,
}

pub struct RexServer {
    state: Arc<AppState>,
    port: u16,
    host: IpAddr,
    static_dir: PathBuf,
    project_root: PathBuf,
}

impl RexServer {
    pub async fn new(config: ServerConfig) -> Self {
        let manifest_json = HotState::compute_manifest_json(&config.build_id, &config.manifest);

        // Compute document descriptor from V8 if _document exists
        let document_descriptor = if config.has_custom_document {
            handlers::compute_document_descriptor(&config.isolate_pool).await
        } else {
            None
        };

        let image_cache = rex_image::ImageCache::new(
            config
                .project_root
                .join(".rex")
                .join("cache")
                .join("images"),
        );

        let state = Arc::new(AppState {
            isolate_pool: config.isolate_pool,
            is_dev: config.is_dev,
            project_root: config.project_root.clone(),
            image_cache,
            esm: None,
            client_deps: std::sync::OnceLock::new(),
            #[cfg(feature = "build")]
            browser_transform_cache: std::sync::OnceLock::new(),
            lazy_init: tokio::sync::OnceCell::const_new_with(()),
            lazy_init_ctx: std::sync::Mutex::new(None),
            hot: RwLock::new(Arc::new(HotState {
                route_trie: config.route_trie,
                api_route_trie: config.api_route_trie,
                has_middleware: config.has_middleware,
                middleware_matchers: config.middleware_matchers,
                manifest: config.manifest,
                build_id: config.build_id,
                has_custom_404: config.has_custom_404,
                has_custom_error: config.has_custom_error,
                has_custom_document: config.has_custom_document,
                project_config: config.project_config,
                manifest_json,
                document_descriptor,
                app_route_trie: config.app_route_trie,
                app_api_route_trie: config.app_api_route_trie,
                has_mcp_tools: config.has_mcp_tools,
                prerendered: std::collections::HashMap::new(),
                prerendered_app: std::collections::HashMap::new(),
                import_map_json: None,
                route_paths: std::collections::HashMap::new(),
            })),
        });

        Self {
            state,
            port: config.port,
            host: config.host,
            static_dir: config.static_dir,
            project_root: config.project_root,
        }
    }

    /// Create a `RexServer` from pre-existing state (used by `Rex` API).
    pub fn from_state(
        state: Arc<AppState>,
        port: u16,
        host: IpAddr,
        static_dir: PathBuf,
        project_root: PathBuf,
    ) -> Self {
        Self {
            state,
            port,
            host,
            static_dir,
            project_root,
        }
    }

    pub fn state(&self) -> Arc<AppState> {
        self.state.clone()
    }

    fn build_router(&self) -> Router {
        self.build_router_with_extra(Router::new())
    }

    pub fn build_router_with_extra(&self, extra_routes: Router<Arc<AppState>>) -> Router {
        let static_service = ServeDir::new(&self.static_dir);

        // Serve public/ directory — uses fallback so unmatched paths fall through to SSR
        let public_dir = self.project_root.join("public");
        let public_service = ServeDir::new(&public_dir).append_index_html_on_directories(false);

        let mut router = Router::new()
            // Data endpoint
            .route("/_rex/data/{build_id}/{*path}", get(handlers::data_handler))
            // Image optimization endpoint
            .route("/_rex/image", get(handlers::image_handler))
            // RSC flight data endpoint (app/ router client navigation)
            .route("/_rex/rsc/{build_id}/{*path}", get(handlers::rsc_handler))
            .route("/_rex/rsc/{build_id}", get(handlers::rsc_handler_root))
            // Server action endpoint (app/ router server functions)
            .route(
                "/_rex/action/{build_id}/{action_id}",
                post(handlers::server_action_handler),
            )
            // MCP endpoint (JSON-RPC 2.0 over POST)
            .route("/mcp", post(crate::mcp::mcp_handler))
            // Client-side router script
            .route("/_rex/router.js", get(router_js_handler))
            // RSC client runtime
            .route("/_rex/rsc-runtime.js", get(rsc_runtime_js_handler))
            // Dev-only: pre-bundled browser deps for unbundled serving
            .route("/_rex/dep/{*specifier}", get(dep_handler));

        // Dev-only routes that require the build feature (OXC transforms, etc.)
        #[cfg(feature = "build")]
        {
            router = router
                .route("/_rex/src/{*path}", get(handlers::src_handler))
                .route("/_rex/entry/{*pattern}", get(handlers::entry_handler));
        }

        router
            // Merge any extra routes (e.g., HMR websocket)
            .merge(extra_routes)
            // Static file serving
            .nest_service("/_rex/static", static_service)
            // API routes: all HTTP methods on /api/*
            .route("/api/{*path}", any(handlers::api_handler))
            // Public directory fallback (before SSR)
            .fallback_service(
                public_service.fallback(handlers::page_handler.with_state(self.state.clone())),
            )
            .with_state(self.state.clone())
            .layer(CompressionLayer::new().gzip(true))
    }

    pub async fn serve(self) -> Result<()> {
        let router = self.build_router();
        let addr = SocketAddr::new(self.host, self.port);

        info!("Rex server listening on http://{addr}");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }
}

async fn router_js_handler() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!(concat!(env!("OUT_DIR"), "/router.js")),
    )
}

async fn rsc_runtime_js_handler() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("../../../runtime/client/rsc-runtime.ts"),
    )
}

/// Serve pre-bundled browser deps for unbundled dev serving.
/// Route: `/_rex/dep/{*specifier}` where specifier is URL-encoded (e.g., `react__jsx-runtime.js`).
async fn dep_handler(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    axum::extract::Path(specifier): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    // Strip .js extension if present
    let key = specifier.strip_suffix(".js").unwrap_or(&specifier);

    if let Some(deps) = state.client_deps.get() {
        if let Some(source) = deps.get(key) {
            return (
                axum::http::StatusCode::OK,
                [
                    (
                        axum::http::header::CONTENT_TYPE,
                        "application/javascript; charset=utf-8",
                    ),
                    (
                        axum::http::header::CACHE_CONTROL,
                        "public, max-age=31536000, immutable",
                    ),
                ],
                source.clone(),
            )
                .into_response();
        }
    }

    (
        axum::http::StatusCode::NOT_FOUND,
        format!("Unknown dep: {key}"),
    )
        .into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::handlers::test_support::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    fn build_dep_app_with_deps(deps: std::collections::HashMap<String, String>) -> Router {
        TestAppBuilder::new()
            .routes(
                vec![make_route("/", "index.tsx", vec![])],
                vec![(
                    "index",
                    "function Index() { return React.createElement('h1', null, 'Home'); }",
                    None,
                )],
            )
            .custom_router(move |state| {
                let _ = state.client_deps.set(Arc::new(deps));
                Router::new()
                    .route("/_rex/dep/{*specifier}", get(dep_handler))
                    .with_state(state)
            })
            .build()
    }

    #[tokio::test]
    async fn dep_handler_serves_known_dep() {
        let mut deps = std::collections::HashMap::new();
        deps.insert(
            "react".to_string(),
            "// react ESM bundle\nexport default {};".to_string(),
        );
        let app = build_dep_app_with_deps(deps);

        let resp = app
            .oneshot(
                Request::get("/_rex/dep/react.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(
            body.contains("react ESM bundle"),
            "Should serve the dep source: {body}"
        );
    }

    #[tokio::test]
    async fn dep_handler_serves_dep_with_immutable_cache() {
        let mut deps = std::collections::HashMap::new();
        deps.insert("react".to_string(), "export default {}".to_string());
        let app = build_dep_app_with_deps(deps);

        let resp = app
            .oneshot(
                Request::get("/_rex/dep/react.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let cc = resp
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            cc.contains("immutable"),
            "Should have immutable cache: {cc}"
        );
        assert!(
            cc.contains("max-age=31536000"),
            "Should have long max-age: {cc}"
        );
    }

    #[tokio::test]
    async fn dep_handler_strips_js_extension() {
        let mut deps = std::collections::HashMap::new();
        deps.insert(
            "react__jsx-runtime".to_string(),
            "export const jsx = () => {};".to_string(),
        );
        let app = build_dep_app_with_deps(deps);

        let resp = app
            .oneshot(
                Request::get("/_rex/dep/react__jsx-runtime.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        assert!(body.contains("jsx"), "Should serve jsx-runtime: {body}");
    }

    #[tokio::test]
    async fn dep_handler_returns_404_for_unknown_dep() {
        let deps = std::collections::HashMap::new();
        let app = build_dep_app_with_deps(deps);

        let resp = app
            .oneshot(
                Request::get("/_rex/dep/unknown-lib.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dep_handler_returns_404_when_no_deps_set() {
        // Build without setting client_deps at all
        let app = TestAppBuilder::new()
            .routes(
                vec![make_route("/", "index.tsx", vec![])],
                vec![(
                    "index",
                    "function Index() { return React.createElement('h1', null, 'Home'); }",
                    None,
                )],
            )
            .custom_router(|state| {
                // Don't set client_deps — OnceLock stays empty
                Router::new()
                    .route("/_rex/dep/{*specifier}", get(dep_handler))
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::get("/_rex/dep/react.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn dep_handler_works_without_js_extension() {
        let mut deps = std::collections::HashMap::new();
        deps.insert("react".to_string(), "export default {}".to_string());
        let app = build_dep_app_with_deps(deps);

        // Request without .js suffix — should use specifier as-is
        let resp = app
            .oneshot(Request::get("/_rex/dep/react").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }
}
