use crate::client_manifest::ClientReferenceManifest;
use rex_core::DataStrategy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Maps route patterns to their client-side asset filenames
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetManifest {
    pub build_id: String,
    /// route pattern -> client chunk filename (pages/ router)
    pub pages: HashMap<String, PageAssets>,
    /// Client _app chunk filename (loaded before page scripts for hydration wrapping)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_script: Option<String>,
    /// Global CSS files (from _app imports), included on every page
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_css: Vec<String>,
    /// CSS file contents for inlining (filename -> content)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub css_contents: HashMap<String, String>,
    /// Shared chunks (e.g. React, common code) split out by rolldown code splitting.
    /// These are modulepreloaded in the HTML head to avoid import waterfalls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_chunks: Vec<String>,
    /// Font files to preload (self-hosted woff2 filenames)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_preloads: Vec<String>,
    /// Middleware matcher patterns extracted from `export const config = { matcher: [...] }`.
    /// None = no middleware, Some(empty) = run on all paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middleware_matchers: Option<Vec<String>>,

    // --- RSC / App Router fields ---
    /// App route patterns -> AppRouteAssets (app/ router)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub app_routes: HashMap<String, AppRouteAssets>,
    /// Client reference manifest for RSC (maps ref IDs to chunk URLs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_reference_manifest: Option<ClientReferenceManifest>,
    /// Path to the RSC server bundle
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsc_server_bundle: Option<String>,
    /// Path to the RSC SSR bundle
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsc_ssr_bundle: Option<String>,
    /// Server action IDs (action_id -> export_name) for discovery
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub server_actions: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageAssets {
    pub js: String,
    /// Per-page CSS files
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub css: Vec<String>,
    /// Data-fetching strategy detected at build time from source exports
    #[serde(default)]
    pub data_strategy: DataStrategy,
}

/// Assets for an app/ route (RSC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRouteAssets {
    /// Client chunks needed for this route's "use client" components
    pub client_chunks: Vec<String>,
    /// Layout chain patterns (for nested rendering)
    pub layout_chain: Vec<String>,
}

impl AssetManifest {
    pub fn new(build_id: String) -> Self {
        Self {
            build_id,
            pages: HashMap::new(),
            app_script: None,
            global_css: Vec::new(),
            css_contents: HashMap::new(),
            shared_chunks: Vec::new(),
            font_preloads: Vec::new(),
            middleware_matchers: None,
            app_routes: HashMap::new(),
            client_reference_manifest: None,
            rsc_server_bundle: None,
            rsc_ssr_bundle: None,
            server_actions: HashMap::new(),
        }
    }

    pub fn add_page(
        &mut self,
        route_pattern: &str,
        js_filename: &str,
        data_strategy: DataStrategy,
    ) {
        self.pages.insert(
            route_pattern.to_string(),
            PageAssets {
                js: js_filename.to_string(),
                css: Vec::new(),
                data_strategy,
            },
        );
    }

    pub fn add_page_with_css(
        &mut self,
        route_pattern: &str,
        js_filename: &str,
        css_filenames: &[String],
        data_strategy: DataStrategy,
    ) {
        self.pages.insert(
            route_pattern.to_string(),
            PageAssets {
                js: js_filename.to_string(),
                css: css_filenames.to_vec(),
                data_strategy,
            },
        );
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}
