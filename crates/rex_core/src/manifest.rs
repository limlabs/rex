use crate::client_manifest::ClientReferenceManifest;
use crate::{DataStrategy, Fallback, RenderMode};
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
    /// Whether this page is statically pre-rendered or server-rendered per request
    #[serde(default)]
    pub render_mode: RenderMode,
    /// Whether this page exports `getStaticPaths` (dynamic routes only)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_static_paths: bool,
    /// Fallback behaviour resolved from `getStaticPaths`
    #[serde(default, skip_serializing_if = "Fallback::is_false")]
    pub fallback: Fallback,
}

/// Assets for an app/ route (RSC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRouteAssets {
    /// Client chunks needed for this route's "use client" components
    pub client_chunks: Vec<String>,
    /// Layout chain patterns (for nested rendering)
    pub layout_chain: Vec<String>,
    /// Render mode for this app route (currently always ServerRendered;
    /// future: auto-detect based on dynamic function usage)
    #[serde(default)]
    pub render_mode: RenderMode,
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
        has_dynamic_segments: bool,
    ) {
        let render_mode = RenderMode::from_strategy(&data_strategy, has_dynamic_segments);
        self.pages.insert(
            route_pattern.to_string(),
            PageAssets {
                js: js_filename.to_string(),
                css: Vec::new(),
                data_strategy,
                render_mode,
                has_static_paths: false,
                fallback: Fallback::default(),
            },
        );
    }

    pub fn add_page_with_css(
        &mut self,
        route_pattern: &str,
        js_filename: &str,
        css_filenames: &[String],
        data_strategy: DataStrategy,
        has_dynamic_segments: bool,
    ) {
        let render_mode = RenderMode::from_strategy(&data_strategy, has_dynamic_segments);
        self.pages.insert(
            route_pattern.to_string(),
            PageAssets {
                js: js_filename.to_string(),
                css: css_filenames.to_vec(),
                data_strategy,
                render_mode,
                has_static_paths: false,
                fallback: Fallback::default(),
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{DataStrategy, RenderMode};

    #[test]
    fn add_page_static_no_dynamic() {
        let mut m = AssetManifest::new("test".into());
        m.add_page("/about", "about.js", DataStrategy::None, false);
        let page = m.pages.get("/about").unwrap();
        assert_eq!(page.js, "about.js");
        assert_eq!(page.render_mode, RenderMode::Static);
    }

    #[test]
    fn add_page_server_rendered_with_gssp() {
        let mut m = AssetManifest::new("test".into());
        m.add_page("/dash", "dash.js", DataStrategy::GetServerSideProps, false);
        assert_eq!(
            m.pages.get("/dash").unwrap().render_mode,
            RenderMode::ServerRendered
        );
    }

    #[test]
    fn add_page_server_rendered_dynamic_segment() {
        let mut m = AssetManifest::new("test".into());
        m.add_page("/blog/:slug", "slug.js", DataStrategy::None, true);
        assert_eq!(
            m.pages.get("/blog/:slug").unwrap().render_mode,
            RenderMode::ServerRendered
        );
    }

    #[test]
    fn add_page_with_css_sets_render_mode() {
        let mut m = AssetManifest::new("test".into());
        m.add_page_with_css(
            "/",
            "index.js",
            &["style.css".into()],
            DataStrategy::GetStaticProps,
            false,
        );
        let page = m.pages.get("/").unwrap();
        assert_eq!(page.css, vec!["style.css"]);
        assert_eq!(page.render_mode, RenderMode::Static);
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = std::env::temp_dir().join("rex_manifest_test.json");
        let mut m = AssetManifest::new("build123".into());
        m.add_page("/", "index.js", DataStrategy::None, false);
        m.save(&tmp).unwrap();

        let loaded = AssetManifest::load(&tmp).unwrap();
        assert_eq!(loaded.build_id, "build123");
        assert_eq!(
            loaded.pages.get("/").unwrap().render_mode,
            RenderMode::Static
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn save_and_load_preserves_all_fields() {
        let tmp = std::env::temp_dir().join("rex_manifest_full_test.json");
        let mut m = AssetManifest::new("full-test".into());
        m.add_page("/", "index.js", DataStrategy::None, false);
        m.add_page("/dash", "dash.js", DataStrategy::GetServerSideProps, false);
        m.add_page_with_css(
            "/styled",
            "styled.js",
            &["style.css".into(), "extra.css".into()],
            DataStrategy::GetStaticProps,
            false,
        );
        m.global_css = vec!["global.css".into()];
        m.shared_chunks = vec!["react-chunk.js".into()];
        m.app_script = Some("_app.js".into());
        m.css_contents
            .insert("style.css".into(), ".foo{color:red}".into());
        m.save(&tmp).unwrap();

        let loaded = AssetManifest::load(&tmp).unwrap();
        assert_eq!(loaded.build_id, "full-test");
        assert_eq!(loaded.pages.len(), 3);
        assert_eq!(
            loaded.pages.get("/").unwrap().render_mode,
            RenderMode::Static
        );
        assert_eq!(
            loaded.pages.get("/dash").unwrap().render_mode,
            RenderMode::ServerRendered
        );
        let styled = loaded.pages.get("/styled").unwrap();
        assert_eq!(styled.render_mode, RenderMode::Static);
        assert_eq!(styled.css, vec!["style.css", "extra.css"]);
        assert_eq!(loaded.global_css, vec!["global.css"]);
        assert_eq!(loaded.shared_chunks, vec!["react-chunk.js"]);
        assert_eq!(loaded.app_script.as_deref(), Some("_app.js"));
        assert_eq!(
            loaded.css_contents.get("style.css").unwrap(),
            ".foo{color:red}"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn new_manifest_has_empty_collections() {
        let m = AssetManifest::new("empty".into());
        assert!(m.pages.is_empty());
        assert!(m.global_css.is_empty());
        assert!(m.shared_chunks.is_empty());
        assert!(m.app_script.is_none());
        assert!(m.css_contents.is_empty());
        assert!(m.middleware_matchers.is_none());
        assert!(m.app_routes.is_empty());
        assert!(m.client_reference_manifest.is_none());
        assert!(m.rsc_server_bundle.is_none());
        assert!(m.rsc_ssr_bundle.is_none());
        assert!(m.server_actions.is_empty());
    }
}
