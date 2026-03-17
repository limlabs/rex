use crate::document::DocumentDescriptor;
use rex_core::{AssetManifest, ProjectConfig, RexConfig};
use rex_router::{RouteTrie, ScanResult};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// State that can change during dev-mode rebuilds.
#[derive(Clone)]
pub struct HotState {
    pub route_trie: RouteTrie,
    pub api_route_trie: RouteTrie,
    pub manifest: AssetManifest,
    pub build_id: String,
    pub has_custom_404: bool,
    pub has_custom_error: bool,
    pub has_custom_document: bool,
    pub project_config: ProjectConfig,
    /// Pre-serialized manifest JSON (build_id + pages), computed once on construction.
    pub manifest_json: String,
    /// Cached document descriptor from _document rendering. None if no custom _document.
    pub document_descriptor: Option<DocumentDescriptor>,
    /// Whether middleware.ts exists in the project root.
    pub has_middleware: bool,
    /// Middleware matcher patterns (None = no middleware, Some(empty) = run on all).
    pub middleware_matchers: Option<Vec<String>>,
    /// App page route trie for app/ router (RSC). None if no app/ directory.
    pub app_route_trie: Option<RouteTrie>,
    /// App API route trie for app/ route handlers (route.ts). None if no route.ts files.
    pub app_api_route_trie: Option<RouteTrie>,
    /// Whether mcp/ directory has tool files.
    pub has_mcp_tools: bool,
    /// Pre-rendered pages for statically optimized routes (route pattern → HTML + props).
    /// Populated at startup for production builds; empty in dev mode.
    pub prerendered: HashMap<String, crate::prerender::PrerenderedPage>,
    /// Pre-rendered app routes (route pattern → HTML + flight data).
    /// Populated at startup for production builds; empty in dev mode.
    pub prerendered_app: HashMap<String, crate::prerender::PrerenderedAppRoute>,
}

impl HotState {
    /// Compute the manifest_json field from current state.
    pub fn compute_manifest_json(build_id: &str, manifest: &AssetManifest) -> String {
        let mut json = serde_json::json!({
            "build_id": build_id,
            "pages": manifest.pages,
        });
        if !manifest.app_routes.is_empty() {
            json["app_routes"] = serde_json::to_value(&manifest.app_routes).unwrap_or_default();
        }
        if !manifest.server_actions.is_empty() {
            json["server_actions"] =
                serde_json::to_value(&manifest.server_actions).unwrap_or_default();
        }
        serde_json::to_string(&json).expect("JSON serialization")
    }
}

/// ESM module loading state for HMR invalidation.
///
/// Persists dep modules and source modules across rebuilds so that
/// the HMR fast path can re-transform a single file and invalidate.
pub struct EsmState {
    pub dep_modules: Arc<Vec<rex_v8::EsmSourceModule>>,
    pub source_modules: Vec<rex_v8::EsmSourceModule>,
    pub entry_specifier: String,
    pub entry_source: String,
}

/// Context for lazy initialization on first request (dev mode only).
pub struct LazyInitContext {
    pub config: RexConfig,
    pub scan: ScanResult,
    pub build_id: String,
    pub pool_size: usize,
}

/// Shared application state
pub struct AppState {
    pub isolate_pool: rex_v8::IsolatePool,
    pub is_dev: bool,
    pub project_root: PathBuf,
    pub image_cache: rex_image::ImageCache,
    pub hot: RwLock<Arc<HotState>>,
    /// ESM module state for HMR fast path. None if using IIFE loading.
    pub esm: Option<RwLock<EsmState>>,
    /// Lazy init gate — first request triggers build + ESM loading (dev mode only).
    pub lazy_init: tokio::sync::OnceCell<()>,
    /// Context needed for lazy init. Consumed on first use.
    pub lazy_init_ctx: std::sync::Mutex<Option<LazyInitContext>>,
}

impl AppState {
    /// Ensure the dev server is fully initialized (build + ESM + V8).
    /// First call does the work; subsequent calls return immediately.
    /// No-op in production mode (everything is initialized eagerly).
    pub async fn ensure_initialized(self: &Arc<Self>) -> anyhow::Result<()> {
        let state = Arc::clone(self);
        self.lazy_init
            .get_or_try_init(|| async {
                let ctx = {
                    let mut guard = state
                        .lazy_init_ctx
                        .lock()
                        .map_err(|e| anyhow::anyhow!("lazy_init_ctx lock poisoned: {e}"))?;
                    match guard.take() {
                        Some(ctx) => ctx,
                        None => return Ok::<(), anyhow::Error>(()), // already consumed
                    }
                };

                tracing::debug!("Lazy init: building bundles + loading ESM...");

                let project_config =
                    ProjectConfig::load(&ctx.config.project_root).unwrap_or_default();
                // Run build and ESM loading in parallel.
                // build_bundles produces CSS/manifest; esm_load_modules loads V8.
                let build_fut = rex_build::build_bundles(&ctx.config, &ctx.scan, &project_config);
                let esm_fut = crate::startup::esm_load_modules(
                    &ctx.config,
                    &ctx.scan,
                    &ctx.build_id,
                    &state.isolate_pool,
                );
                let (build_result, esm_state) = tokio::try_join!(build_fut, esm_fut)?;

                tracing::debug!(
                    build_id = %build_result.build_id,
                    "Lazy init complete"
                );

                // Update HotState with real manifest
                if let Ok(mut guard) = state.hot.write() {
                    let mut hot = (**guard).clone();
                    hot.manifest = build_result.manifest;
                    hot.build_id = build_result.build_id.clone();
                    hot.manifest_json =
                        HotState::compute_manifest_json(&hot.build_id, &hot.manifest);
                    *guard = Arc::new(hot);
                }

                // Set ESM state for HMR
                if let Some(esm_lock) = &state.esm {
                    if let Ok(mut guard) = esm_lock.write() {
                        *guard = esm_state;
                    }
                }

                Ok(())
            })
            .await?;
        Ok(())
    }
}

/// Snapshot the hot state (O(1) Arc clone, no lock held across await).
pub fn snapshot(state: &Arc<AppState>) -> Arc<HotState> {
    Arc::clone(&state.hot.read().expect("HotState lock poisoned"))
}
