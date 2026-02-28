use crate::handlers::{self, AppState, HotState};
use anyhow::Result;
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

pub struct RexServer {
    state: Arc<AppState>,
    port: u16,
    static_dir: PathBuf,
}

impl RexServer {
    pub async fn with_error_pages(
        route_trie: RouteTrie,
        api_route_trie: RouteTrie,
        isolate_pool: IsolatePool,
        manifest: AssetManifest,
        build_id: String,
        static_dir: PathBuf,
        port: u16,
        is_dev: bool,
        has_custom_404: bool,
        has_custom_error: bool,
        has_custom_document: bool,
        project_config: ProjectConfig,
    ) -> Self {
        let manifest_json = HotState::compute_manifest_json(&build_id, &manifest);

        // Compute document descriptor from V8 if _document exists
        let document_descriptor = if has_custom_document {
            handlers::compute_document_descriptor(&isolate_pool).await
        } else {
            None
        };

        let state = Arc::new(AppState {
            isolate_pool,
            is_dev,
            hot: RwLock::new(Arc::new(HotState {
                route_trie,
                api_route_trie,
                manifest,
                build_id,
                has_custom_404,
                has_custom_error,
                has_custom_document,
                project_config,
                manifest_json,
                document_descriptor,
            })),
        });

        Self {
            state,
            port,
            static_dir,
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

        Router::new()
            // Data endpoint
            .route(
                "/_rex/data/{build_id}/{*path}",
                get(handlers::data_handler),
            )
            // Client-side router script
            .route("/_rex/router.js", get(router_js_handler))
            // Merge any extra routes (e.g., HMR websocket)
            .merge(extra_routes)
            // Static file serving
            .nest_service("/_rex/static", static_service)
            // API routes: all HTTP methods on /api/*
            .route("/api/{*path}", any(handlers::api_handler))
            // Fallback: all other routes go through SSR
            .fallback(handlers::page_handler)
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
