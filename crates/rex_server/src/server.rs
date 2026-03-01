use crate::handlers::{self, AppState, HotState};
use anyhow::Result;
use axum::handler::Handler;
use axum::routing::{any, get};
use axum::Router;
use rex_build::AssetManifest;
use rex_core::ProjectConfig;
use rex_router::RouteTrie;
use rex_v8::IsolatePool;
use std::net::SocketAddr;
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
}

pub struct RexServer {
    state: Arc<AppState>,
    port: u16,
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
            config.project_root.join(".rex").join("cache").join("images"),
        );

        let state = Arc::new(AppState {
            isolate_pool: config.isolate_pool,
            is_dev: config.is_dev,
            project_root: config.project_root.clone(),
            image_cache,
            hot: RwLock::new(Arc::new(HotState {
                route_trie: config.route_trie,
                api_route_trie: config.api_route_trie,
                manifest: config.manifest,
                build_id: config.build_id,
                has_custom_404: config.has_custom_404,
                has_custom_error: config.has_custom_error,
                has_custom_document: config.has_custom_document,
                project_config: config.project_config,
                manifest_json,
                document_descriptor,
            })),
        });

        Self {
            state,
            port: config.port,
            static_dir: config.static_dir,
            project_root: config.project_root,
        }
    }

    /// Create a `RexServer` from pre-existing state (used by `Rex` API).
    pub fn from_state(
        state: Arc<AppState>,
        port: u16,
        static_dir: PathBuf,
        project_root: PathBuf,
    ) -> Self {
        Self {
            state,
            port,
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

        Router::new()
            // Data endpoint
            .route(
                "/_rex/data/{build_id}/{*path}",
                get(handlers::data_handler),
            )
            // Image optimization endpoint
            .route("/_rex/image", get(handlers::image_handler))
            // Client-side router script
            .route("/_rex/router.js", get(router_js_handler))
            // Merge any extra routes (e.g., HMR websocket)
            .merge(extra_routes)
            // Static file serving
            .nest_service("/_rex/static", static_service)
            // API routes: all HTTP methods on /api/*
            .route("/api/{*path}", any(handlers::api_handler))
            // Public directory fallback (before SSR)
            .fallback_service(
                public_service
                    .fallback(handlers::page_handler.with_state(self.state.clone())),
            )
            .with_state(self.state.clone())
            .layer(CompressionLayer::new().gzip(true))
    }

    pub async fn serve(self) -> Result<()> {
        let router = self.build_router();
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));

        info!("Rex server listening on http://{addr}");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }
}

async fn router_js_handler() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("../../../runtime/client/router.js"),
    )
}
