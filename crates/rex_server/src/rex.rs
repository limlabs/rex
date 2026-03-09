use crate::core::{self, body_to_string, RexRequest, RexResponse, RouteMatchResult};
use crate::server::RexServer;
use crate::state::{snapshot, AppState, HotState};
use anyhow::Result;
use axum::Router;
use rex_core::{AssetManifest, DataStrategy, ProjectConfig, RexConfig, ServerSidePropsContext};
use rex_router::{scan_project, RouteTrie, ScanResult};
use rex_v8::{init_v8, IsolatePool};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::debug;

/// Options for creating a Rex instance.
#[derive(Debug, Clone)]
pub struct RexOptions {
    /// Path to the project root directory (containing `pages/`).
    pub root: PathBuf,
    /// Whether to run in dev mode (enables HMR, error overlays).
    pub dev: bool,
    /// Port to listen on (used by `serve()`).
    pub port: u16,
    /// Host/IP address to bind to (e.g. 127.0.0.1 for local, 0.0.0.0 for all interfaces).
    pub host: IpAddr,
}

impl Default for RexOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            dev: false,
            port: 3000,
            host: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        }
    }
}

/// Result of rendering a full page (GSSP + SSR + document assembly).
#[derive(Debug, Clone)]
pub struct PageResult {
    /// Full HTML document string.
    pub html: String,
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
}

/// The main Rex API struct.
///
/// Encapsulates the full init pipeline (scan → build → V8 → pool → server)
/// and exposes methods for route matching, SSR, and request handling.
///
/// # Standalone server
/// ```no_run
/// # async fn example() -> anyhow::Result<()> {
/// use rex_server::Rex;
/// use rex_server::RexOptions;
///
/// let rex = Rex::new(RexOptions { root: ".".into(), ..Default::default() }).await?;
/// rex.serve().await?;
/// # Ok(())
/// # }
/// ```
///
/// # Custom Axum server
/// ```no_run
/// # async fn example() -> anyhow::Result<()> {
/// use axum::routing::get;
/// use axum::Router;
/// use rex_server::Rex;
/// use rex_server::RexOptions;
///
/// let rex = Rex::new(RexOptions { root: ".".into(), ..Default::default() }).await?;
/// let app = Router::new()
///     .route("/healthz", get(|| async { "ok" }))
///     .merge(rex.router());
/// let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
/// axum::serve(listener, app).await?;
/// # Ok(())
/// # }
/// ```
pub struct Rex {
    state: Arc<AppState>,
    config: RexConfig,
    static_dir: PathBuf,
    scan: ScanResult,
    port: u16,
    host: IpAddr,
}

impl Rex {
    /// Create a new Rex instance by scanning pages, building bundles, and initializing V8.
    ///
    /// This is the primary constructor for dev mode and fresh builds.
    /// Requires the `build` feature (pulls in the rolldown bundler).
    #[cfg(feature = "build")]
    pub async fn new(opts: RexOptions) -> Result<Self> {
        let root = std::fs::canonicalize(&opts.root)?;
        let config = RexConfig::new(root).with_dev(opts.dev).with_port(opts.port);
        config.validate()?;

        let project_config = ProjectConfig::load(&config.project_root)?;

        // Scan pages + middleware
        debug!("Scanning routes...");
        let scan = scan_project(&config.project_root, &config.pages_dir)?;
        debug!(
            routes = scan.routes.len(),
            has_app = scan.app.is_some(),
            has_404 = scan.not_found.is_some(),
            has_error = scan.error.is_some(),
            "Routes scanned"
        );

        // Build bundles
        debug!("Building bundles...");
        let build_result = rex_build::build_bundles(&config, &scan, &project_config).await?;
        debug!(build_id = %build_result.build_id, "Build complete");

        // Initialize V8
        debug!("Initializing V8...");
        init_v8();

        let mut server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)?;

        // If RSC bundles exist, append them so RSC functions are available in V8
        if let Some(rsc_path) = &build_result.manifest.rsc_server_bundle {
            let rsc_bundle = std::fs::read_to_string(rsc_path)?;
            server_bundle.push_str("\n;\n");
            server_bundle.push_str(&rsc_bundle);
            debug!("RSC flight bundle appended to V8 bundle");
        }
        if let Some(ssr_path) = &build_result.manifest.rsc_ssr_bundle {
            let ssr_bundle = std::fs::read_to_string(ssr_path)?;
            server_bundle.push_str("\n;\n");
            server_bundle.push_str(&ssr_bundle);
            debug!("RSC SSR bundle appended to V8 bundle");
        }

        let pool_size = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(if opts.dev { 4 } else { 8 });

        debug!(pool_size, "Creating V8 isolate pool");
        let project_root_str = config.project_root.to_string_lossy().to_string();
        let pool = IsolatePool::new(
            pool_size,
            Arc::new(server_bundle),
            Some(Arc::new(project_root_str)),
        )?;

        let static_dir = config.client_build_dir();

        Self::init_from_parts(
            config,
            scan,
            pool,
            build_result.manifest,
            build_result.build_id,
            static_dir,
            project_config,
            opts.port,
            opts.host,
        )
        .await
    }

    /// Create a Rex instance from a pre-built manifest (for `rex start`).
    ///
    /// Skips the build step and loads the manifest from disk.
    pub async fn from_build(opts: RexOptions) -> Result<Self> {
        let root = std::fs::canonicalize(&opts.root)?;
        let config = RexConfig::new(root).with_port(opts.port);

        // Load manifest
        let manifest = AssetManifest::load(&config.manifest_path())?;

        // Scan routes + middleware (for trie)
        let scan = scan_project(&config.project_root, &config.pages_dir)?;

        // Initialize V8
        init_v8();

        let mut server_bundle = std::fs::read_to_string(config.server_bundle_path())?;

        // If RSC bundles exist, append them
        if let Some(rsc_path) = &manifest.rsc_server_bundle {
            if let Ok(rsc_bundle) = std::fs::read_to_string(rsc_path) {
                server_bundle.push_str("\n;\n");
                server_bundle.push_str(&rsc_bundle);
            }
        }
        if let Some(ssr_path) = &manifest.rsc_ssr_bundle {
            if let Ok(ssr_bundle) = std::fs::read_to_string(ssr_path) {
                server_bundle.push_str("\n;\n");
                server_bundle.push_str(&ssr_bundle);
            }
        }

        let pool_size = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        let project_root_str = config.project_root.to_string_lossy().to_string();
        let pool = IsolatePool::new(
            pool_size,
            Arc::new(server_bundle),
            Some(Arc::new(project_root_str)),
        )?;

        let static_dir = config.client_build_dir();
        let build_id = manifest.build_id.clone();
        let project_config = ProjectConfig::load(&config.project_root)?;

        Self::init_from_parts(
            config,
            scan,
            pool,
            manifest,
            build_id,
            static_dir,
            project_config,
            opts.port,
            opts.host,
        )
        .await
    }

    /// Shared initialization logic: builds route tries, computes document descriptor,
    /// pre-renders static pages, and creates AppState and HotState.
    #[allow(clippy::too_many_arguments)]
    async fn init_from_parts(
        config: RexConfig,
        scan: ScanResult,
        pool: IsolatePool,
        manifest: AssetManifest,
        build_id: String,
        static_dir: PathBuf,
        project_config: ProjectConfig,
        port: u16,
        host: IpAddr,
    ) -> Result<Self> {
        let trie = RouteTrie::from_routes(&scan.routes);
        let api_trie = RouteTrie::from_routes(&scan.api_routes);
        let manifest_json = HotState::compute_manifest_json(&build_id, &manifest);

        // Build app route trie from app scan if present
        let app_route_trie = scan.app_scan.as_ref().map(|app| {
            let routes = app.to_routes();
            debug!(app_routes = routes.len(), "Building app route trie");
            RouteTrie::from_routes(&routes)
        });

        // Build app API route trie from app scan if route.ts files exist
        let app_api_route_trie = scan.app_scan.as_ref().and_then(|app| {
            if app.api_routes.is_empty() {
                None
            } else {
                let routes = app.to_api_routes();
                debug!(app_api_routes = routes.len(), "Building app API route trie");
                Some(RouteTrie::from_routes(&routes))
            }
        });

        // Compute document descriptor if custom _document exists
        let document_descriptor = if scan.document.is_some() {
            crate::handlers::compute_document_descriptor(&pool).await
        } else {
            None
        };

        // Automatic static optimization: pre-render eligible pages at startup.
        // In dev mode, skip pre-rendering so pages always reflect the latest code.
        let (prerendered, prerendered_app) = if !config.dev {
            let pages = crate::prerender::prerender_static_pages(
                &pool,
                &manifest,
                &scan.routes,
                &manifest_json,
                document_descriptor.as_ref(),
            )
            .await;
            let app =
                crate::prerender::prerender_static_app_routes(&pool, &manifest, &manifest_json)
                    .await;
            (pages, app)
        } else {
            (HashMap::new(), HashMap::new())
        };

        let image_cache = rex_image::ImageCache::new(
            config
                .project_root
                .join(".rex")
                .join("cache")
                .join("images"),
        );

        let state = Arc::new(AppState {
            isolate_pool: pool,
            is_dev: config.dev,
            project_root: config.project_root.clone(),
            image_cache,
            hot: RwLock::new(Arc::new(HotState {
                route_trie: trie,
                api_route_trie: api_trie,
                has_middleware: scan.middleware.is_some(),
                middleware_matchers: manifest.middleware_matchers.clone(),
                manifest,
                build_id,
                has_custom_404: scan.not_found.is_some(),
                has_custom_error: scan.error.is_some(),
                has_custom_document: scan.document.is_some(),
                project_config,
                manifest_json,
                document_descriptor,
                app_route_trie,
                app_api_route_trie,
                has_mcp_tools: !scan.mcp_tools.is_empty(),
                prerendered,
                prerendered_app,
            })),
        });

        Ok(Self {
            state,
            config,
            static_dir,
            scan,
            port,
            host,
        })
    }

    // --- Accessors ---

    /// Whether this instance is running in dev mode.
    pub fn is_dev(&self) -> bool {
        self.config.dev
    }

    /// The current build ID.
    pub fn build_id(&self) -> String {
        snapshot(&self.state).build_id.clone()
    }

    /// Path to the client-side static files directory.
    pub fn static_dir(&self) -> &PathBuf {
        &self.static_dir
    }

    /// Path to the project root directory.
    pub fn project_root(&self) -> &PathBuf {
        &self.config.project_root
    }

    /// The shared `AppState` — useful for `rex_dev` rebuild handler and custom integrations.
    pub fn state(&self) -> Arc<AppState> {
        self.state.clone()
    }

    /// The `RexConfig` used to initialize this instance.
    pub fn config(&self) -> &RexConfig {
        &self.config
    }

    /// The scan result from the pages directory.
    pub fn scan(&self) -> &ScanResult {
        &self.scan
    }

    // --- Route matching ---

    /// Match a URL path against the page route trie.
    pub fn match_route(&self, path: &str) -> Option<RouteMatchResult> {
        let hot = snapshot(&self.state);
        core::match_route(&hot, path)
    }

    // --- Data fetching ---

    /// Execute getServerSideProps (or getStaticProps) for a path and return the raw JSON result.
    pub async fn get_server_side_props(&self, path: &str) -> Result<serde_json::Value> {
        let hot = snapshot(&self.state);

        let route_match = hot
            .route_trie
            .match_path(path)
            .ok_or_else(|| anyhow::anyhow!("No route matches path: {path}"))?;

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
            DataStrategy::None => Ok(Ok(r#"{"props":{}}"#.to_string())),
            DataStrategy::GetStaticProps => {
                let ctx_json = serde_json::json!({ "params": params }).to_string();
                self.state
                    .isolate_pool
                    .execute(move |iso| iso.get_static_props(&route_key, &ctx_json))
                    .await
            }
            DataStrategy::GetServerSideProps => {
                let context = ServerSidePropsContext {
                    params,
                    query: HashMap::new(),
                    resolved_url: path.to_string(),
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
            Ok(Err(e)) => anyhow::bail!("GSSP error: {e}"),
            Err(e) => anyhow::bail!("Pool error: {e}"),
        };

        Ok(serde_json::from_str(&json_str)?)
    }

    // --- Rendering ---

    /// Render a page to an HTML body string with the given props (SSR only, no GSSP).
    pub async fn render_to_string(&self, path: &str, props: &serde_json::Value) -> Result<String> {
        let hot = snapshot(&self.state);

        let route_match = hot
            .route_trie
            .match_path(path)
            .ok_or_else(|| anyhow::anyhow!("No route matches path: {path}"))?;

        let route_key = route_match.route.module_name();
        let props_json = serde_json::to_string(props).expect("JSON serialization");

        let result = self
            .state
            .isolate_pool
            .execute(move |iso| iso.render_page(&route_key, &props_json))
            .await;

        match result {
            Ok(Ok(r)) => Ok(r.body),
            Ok(Err(e)) => anyhow::bail!("SSR render error: {e}"),
            Err(e) => anyhow::bail!("Pool error: {e}"),
        }
    }

    /// Render a full page: run GSSP, SSR, and assemble the HTML document.
    pub async fn render_page(&self, path: &str) -> Result<PageResult> {
        let hot = snapshot(&self.state);
        let req = RexRequest {
            method: "GET".to_string(),
            path: path.to_string(),
            query: None,
            headers: HashMap::new(),
            body: Vec::new(),
        };

        let resp = core::handle_page(&self.state, &hot, &req).await;

        Ok(PageResult {
            html: body_to_string(&resp.body),
            status: resp.status,
            headers: resp.headers,
        })
    }

    // --- Request handling ---

    /// Handle a framework-agnostic request and return a framework-agnostic response.
    ///
    /// Routes to pages, API endpoints, data endpoints, or image optimization
    /// based on the request path. Does NOT serve static files from `/_rex/static/`.
    pub async fn handle_request(&self, req: &RexRequest) -> RexResponse {
        let hot = snapshot(&self.state);
        core::handle_request(&self.state, &hot, req).await
    }

    // --- Axum integration ---

    /// Build an Axum `Router` with all Rex routes (pages, API, data, static, images).
    pub fn router(&self) -> Router {
        let server = self.build_rex_server();
        server.build_router_with_extra(Router::new())
    }

    /// Build an Axum `Router` with Rex routes merged with custom extra routes.
    pub fn router_with_extra(&self, extra: Router<Arc<AppState>>) -> Router {
        let server = self.build_rex_server();
        server.build_router_with_extra(extra)
    }

    /// Bind to the configured host and port and serve.
    pub async fn serve(self) -> Result<()> {
        let router = self.router();
        let addr = std::net::SocketAddr::new(self.host, self.port);

        tracing::info!("Rex server listening on http://{addr}");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }

    /// Internal: build a RexServer from our state (reuses existing router logic).
    fn build_rex_server(&self) -> RexServer {
        RexServer::from_state(
            self.state.clone(),
            self.port,
            self.host,
            self.static_dir.clone(),
            self.config.project_root.clone(),
        )
    }
}
