use crate::handlers::{self, AppState};
use anyhow::Result;
use axum::routing::{any, get};
use axum::Router;
use rex_build::AssetManifest;
use rex_router::RouteTrie;
use rex_v8::IsolatePool;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tracing::info;

pub struct RexServer {
    state: Arc<AppState>,
    port: u16,
    static_dir: PathBuf,
}

impl RexServer {
    pub fn new(
        route_trie: RouteTrie,
        isolate_pool: IsolatePool,
        manifest: AssetManifest,
        build_id: String,
        static_dir: PathBuf,
        port: u16,
        is_dev: bool,
    ) -> Self {
        Self::with_error_pages(route_trie, RouteTrie::from_routes(&[]), isolate_pool, manifest, build_id, static_dir, port, is_dev, false, false)
    }

    pub fn with_error_pages(
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
    ) -> Self {
        let state = Arc::new(AppState {
            route_trie,
            api_route_trie,
            isolate_pool,
            manifest,
            build_id,
            is_dev,
            has_custom_404,
            has_custom_error,
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

    pub fn build_router(&self) -> Router {
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
            // Merge any extra routes (e.g., HMR websocket)
            .merge(extra_routes)
            // Static file serving
            .nest_service("/_rex/static", static_service)
            // API routes: all HTTP methods on /api/*
            .route("/api/{*path}", any(handlers::api_handler))
            // Fallback: all other routes go through SSR
            .fallback(handlers::page_handler)
            .with_state(self.state.clone())
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
