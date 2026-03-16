use crate::document::DocumentDescriptor;
use rex_core::{AssetManifest, ProjectConfig};
use rex_router::RouteTrie;
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

/// Shared application state
pub struct AppState {
    pub isolate_pool: rex_v8::IsolatePool,
    pub is_dev: bool,
    pub project_root: PathBuf,
    pub image_cache: rex_image::ImageCache,
    pub hot: RwLock<Arc<HotState>>,
    /// ESM module state for HMR fast path. None if using IIFE loading.
    pub esm: Option<RwLock<EsmState>>,
}

/// Snapshot the hot state (O(1) Arc clone, no lock held across await).
pub fn snapshot(state: &Arc<AppState>) -> Arc<HotState> {
    Arc::clone(&state.hot.read().expect("HotState lock poisoned"))
}
