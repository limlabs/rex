use crate::document::{assemble_body_tail, assemble_head_shell, DocumentDescriptor};
use rex_build::AssetManifest;
use rex_core::RenderMode;
use rex_v8::IsolatePool;
use std::collections::HashMap;
use tracing::{debug, info, warn};

use rex_build::manifest::PageAssets;

/// Pre-render all statically optimized pages and return a map of route pattern -> full HTML.
///
/// Pages are eligible for static optimization when their `render_mode` is `Static`:
/// - No `getServerSideProps` export
/// - No dynamic route segments (e.g., `[slug]`)
/// - Either no data function or `getStaticProps` (called at build/startup time)
pub async fn prerender_static_pages(
    pool: &IsolatePool,
    manifest: &AssetManifest,
    manifest_json: &str,
    doc_descriptor: Option<&DocumentDescriptor>,
) -> HashMap<String, String> {
    let mut prerendered = HashMap::new();

    let static_pages = collect_static_pages(manifest);

    for (pattern, assets) in &static_pages {
        let route_key = pattern_to_module_name(pattern);

        // Get props: either empty or from getStaticProps
        let props_json = match &assets.data_strategy {
            rex_core::DataStrategy::GetStaticProps => {
                let key = route_key.clone();
                let ctx = serde_json::json!({ "params": {} }).to_string();
                match pool
                    .execute(move |iso| iso.get_static_props(&key, &ctx))
                    .await
                {
                    Ok(Ok(json)) => extract_props_from_gsp(&json),
                    Ok(Err(e)) => {
                        warn!(pattern, error = %e, "getStaticProps failed, skipping pre-render");
                        continue;
                    }
                    Err(e) => {
                        warn!(pattern, error = %e, "Pool error during getStaticProps");
                        continue;
                    }
                }
            }
            _ => "{}".to_string(),
        };

        // Render the page via V8
        let key = route_key.clone();
        let props = props_json.clone();
        let render_result = match pool.execute(move |iso| iso.render_page(&key, &props)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                warn!(pattern, error = %e, "SSR render failed, skipping pre-render");
                continue;
            }
            Err(e) => {
                warn!(pattern, error = %e, "Pool error during render");
                continue;
            }
        };

        let html = assemble_static_html(
            &render_result,
            assets,
            manifest,
            &props_json,
            manifest_json,
            doc_descriptor,
        );
        debug!(pattern, bytes = html.len(), "Pre-rendered static page");
        prerendered.insert(pattern.clone(), html);
    }

    if !prerendered.is_empty() {
        info!(
            count = prerendered.len(),
            "Static pages pre-rendered (automatic static optimization)"
        );
    }

    prerendered
}

/// Collect pages eligible for static pre-rendering from the manifest.
fn collect_static_pages(manifest: &AssetManifest) -> Vec<(String, PageAssets)> {
    manifest
        .pages
        .iter()
        .filter(|(_, assets)| assets.render_mode == RenderMode::Static)
        .map(|(pattern, assets)| (pattern.clone(), assets.clone()))
        .collect()
}

/// Extract just the `props` value from a getStaticProps result JSON like `{ "props": {...} }`.
fn extract_props_from_gsp(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(val) => {
            if let Some(props) = val.get("props") {
                serde_json::to_string(props).unwrap_or_else(|_| "{}".into())
            } else {
                "{}".to_string()
            }
        }
        Err(_) => "{}".to_string(),
    }
}

/// Assemble a full HTML document for a statically pre-rendered page.
fn assemble_static_html(
    render_result: &rex_v8::RenderResult,
    assets: &PageAssets,
    manifest: &AssetManifest,
    props_json: &str,
    manifest_json: &str,
    doc_descriptor: Option<&DocumentDescriptor>,
) -> String {
    let client_scripts: Vec<String> = vec![assets.js.clone()];

    let mut css_files = manifest.global_css.clone();
    css_files.extend(assets.css.iter().cloned());

    let shell = assemble_head_shell(
        &css_files,
        &manifest.css_contents,
        &manifest.shared_chunks,
        manifest.app_script.as_deref(),
        &client_scripts,
        doc_descriptor,
    );

    let tail = assemble_body_tail(
        &render_result.body,
        &render_result.head,
        props_json,
        &client_scripts,
        manifest.app_script.as_deref(),
        false, // never dev mode for pre-rendered pages
        Some(manifest_json),
    );

    format!("{shell}{tail}")
}

/// Convert a route pattern like "/" or "/about" to a module name like "index" or "about".
///
/// Module names are how pages are registered in the V8 `__rex_pages` object.
fn pattern_to_module_name(pattern: &str) -> String {
    let trimmed = pattern.trim_start_matches('/');
    if trimmed.is_empty() {
        "index".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::DataStrategy;

    #[test]
    fn pattern_to_module_name_root() {
        assert_eq!(pattern_to_module_name("/"), "index");
    }

    #[test]
    fn pattern_to_module_name_path() {
        assert_eq!(pattern_to_module_name("/about"), "about");
    }

    #[test]
    fn pattern_to_module_name_nested() {
        assert_eq!(pattern_to_module_name("/blog/posts"), "blog/posts");
    }

    #[test]
    fn extract_props_with_props_key() {
        let json = r#"{"props":{"title":"Hello","count":42}}"#;
        let result = extract_props_from_gsp(json);
        let val: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(val["title"], "Hello");
        assert_eq!(val["count"], 42);
    }

    #[test]
    fn extract_props_missing_props_key() {
        let json = r#"{"data":"something"}"#;
        assert_eq!(extract_props_from_gsp(json), "{}");
    }

    #[test]
    fn extract_props_invalid_json() {
        assert_eq!(extract_props_from_gsp("not json"), "{}");
    }

    #[test]
    fn extract_props_empty_props() {
        let json = r#"{"props":{}}"#;
        assert_eq!(extract_props_from_gsp(json), "{}");
    }

    #[test]
    fn collect_static_pages_filters_correctly() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/", "index.js", DataStrategy::None, false);
        manifest.add_page("/about", "about.js", DataStrategy::None, false);
        manifest.add_page("/dash", "dash.js", DataStrategy::GetServerSideProps, false);
        manifest.add_page("/blog/:slug", "slug.js", DataStrategy::None, true);

        let static_pages = collect_static_pages(&manifest);
        let patterns: Vec<&str> = static_pages
            .iter()
            .map(|(p, _): &(String, PageAssets)| p.as_str())
            .collect();

        assert_eq!(static_pages.len(), 2);
        assert!(patterns.contains(&"/"));
        assert!(patterns.contains(&"/about"));
    }

    #[test]
    fn collect_static_pages_empty_manifest() {
        let manifest = AssetManifest::new("test".into());
        assert!(collect_static_pages(&manifest).is_empty());
    }

    #[test]
    fn collect_static_pages_all_server_rendered() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/", "index.js", DataStrategy::GetServerSideProps, false);
        manifest.add_page("/blog/:slug", "slug.js", DataStrategy::None, true);
        assert!(collect_static_pages(&manifest).is_empty());
    }

    #[test]
    fn assemble_static_html_produces_valid_document() {
        let render_result = rex_v8::RenderResult {
            body: "<div>Hello</div>".to_string(),
            head: "<title>Test</title>".to_string(),
        };
        let mut manifest = AssetManifest::new("build123".into());
        manifest.add_page("/", "index.js", DataStrategy::None, false);
        let assets = manifest.pages.get("/").unwrap();
        let manifest_json = serde_json::to_string(&manifest).unwrap();

        let html = assemble_static_html(
            &render_result,
            assets,
            &manifest,
            "{}",
            &manifest_json,
            None,
        );

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<div>Hello</div>"));
        assert!(html.contains("<title>Test</title>"));
        assert!(html.contains("index.js"));
    }

    #[test]
    fn assemble_static_html_includes_css() {
        let render_result = rex_v8::RenderResult {
            body: "<p>Styled</p>".to_string(),
            head: String::new(),
        };
        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page_with_css(
            "/styled",
            "styled.js",
            &["page.css".into()],
            DataStrategy::None,
            false,
        );
        manifest.global_css = vec!["global.css".into()];
        let assets = manifest.pages.get("/styled").unwrap();
        let manifest_json = serde_json::to_string(&manifest).unwrap();

        let html = assemble_static_html(
            &render_result,
            assets,
            &manifest,
            "{}",
            &manifest_json,
            None,
        );

        assert!(html.contains("global.css"));
        assert!(html.contains("page.css"));
        assert!(html.contains("<p>Styled</p>"));
    }
}
