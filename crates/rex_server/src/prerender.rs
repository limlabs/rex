use crate::document::{assemble_body_tail, assemble_head_shell, DocumentDescriptor};
use rex_build::manifest::PageAssets;
use rex_build::AssetManifest;
use rex_core::{RenderMode, Route};
use rex_v8::IsolatePool;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Pre-render all statically optimized pages and return a map of route pattern -> full HTML.
///
/// Pages are eligible for static optimization when their `render_mode` is `Static`:
/// - No `getServerSideProps` export
/// - No dynamic route segments (e.g., `[slug]`)
/// - Either no data function or `getStaticProps` (called at build/startup time)
pub async fn prerender_static_pages(
    pool: &IsolatePool,
    manifest: &AssetManifest,
    routes: &[Route],
    manifest_json: &str,
    doc_descriptor: Option<&DocumentDescriptor>,
) -> HashMap<String, String> {
    let mut prerendered = HashMap::new();

    // Build a lookup from route pattern -> Route so we can use module_name()
    let route_by_pattern: HashMap<&str, &Route> =
        routes.iter().map(|r| (r.pattern.as_str(), r)).collect();

    let static_pages = collect_static_pages(manifest);

    for (pattern, assets) in &static_pages {
        // Use Route::module_name() for the V8 registry key, which correctly
        // handles nested index pages (e.g., pages/blog/index.tsx -> "blog/index").
        let route_key = match route_by_pattern.get(pattern.as_str()) {
            Some(route) => route.module_name(),
            None => {
                warn!(pattern, "No route found for static page, skipping");
                continue;
            }
        };

        // Get props: either empty or from getStaticProps
        let props_json = match &assets.data_strategy {
            rex_core::DataStrategy::GetStaticProps => {
                let key = route_key.clone();
                let ctx = serde_json::json!({ "params": {} }).to_string();
                match pool
                    .execute(move |iso| iso.get_static_props(&key, &ctx))
                    .await
                {
                    Ok(Ok(json)) => match parse_gsp_result(&json) {
                        GspOutcome::Props(props) => props,
                        GspOutcome::Redirect { destination } => {
                            debug!(
                                pattern,
                                destination,
                                "getStaticProps returned redirect, skipping pre-render"
                            );
                            continue;
                        }
                        GspOutcome::NotFound => {
                            debug!(
                                pattern,
                                "getStaticProps returned notFound, skipping pre-render"
                            );
                            continue;
                        }
                    },
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

/// Outcome of parsing a getStaticProps result.
#[derive(Debug, PartialEq)]
enum GspOutcome {
    /// Normal props to render with
    Props(String),
    /// GSP returned `{ redirect: { destination: "..." } }`
    Redirect { destination: String },
    /// GSP returned `{ notFound: true }`
    NotFound,
}

/// Parse a getStaticProps result JSON, handling `props`, `redirect`, and `notFound`.
fn parse_gsp_result(json: &str) -> GspOutcome {
    let val = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(v) => v,
        Err(_) => return GspOutcome::Props("{}".to_string()),
    };

    // Check for redirect
    if let Some(redirect) = val.get("redirect") {
        let destination = redirect
            .get("destination")
            .and_then(|d| d.as_str())
            .unwrap_or("/")
            .to_string();
        return GspOutcome::Redirect { destination };
    }

    // Check for notFound
    if val
        .get("notFound")
        .and_then(|n| n.as_bool())
        .unwrap_or(false)
    {
        return GspOutcome::NotFound;
    }

    // Extract props
    if let Some(props) = val.get("props") {
        GspOutcome::Props(serde_json::to_string(props).unwrap_or_else(|_| "{}".into()))
    } else {
        GspOutcome::Props("{}".to_string())
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
        &manifest.font_preloads,
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::DataStrategy;

    #[test]
    fn parse_gsp_result_with_props() {
        let json = r#"{"props":{"title":"Hello","count":42}}"#;
        match parse_gsp_result(json) {
            GspOutcome::Props(props) => {
                let val: serde_json::Value = serde_json::from_str(&props).unwrap();
                assert_eq!(val["title"], "Hello");
                assert_eq!(val["count"], 42);
            }
            other => panic!("Expected Props, got {other:?}"),
        }
    }

    #[test]
    fn parse_gsp_result_missing_props_key() {
        assert_eq!(
            parse_gsp_result(r#"{"data":"something"}"#),
            GspOutcome::Props("{}".to_string())
        );
    }

    #[test]
    fn parse_gsp_result_invalid_json() {
        assert_eq!(
            parse_gsp_result("not json"),
            GspOutcome::Props("{}".to_string())
        );
    }

    #[test]
    fn parse_gsp_result_empty_props() {
        assert_eq!(
            parse_gsp_result(r#"{"props":{}}"#),
            GspOutcome::Props("{}".to_string())
        );
    }

    #[test]
    fn parse_gsp_result_redirect() {
        let json = r#"{"redirect":{"destination":"/login","permanent":false}}"#;
        assert_eq!(
            parse_gsp_result(json),
            GspOutcome::Redirect {
                destination: "/login".to_string()
            }
        );
    }

    #[test]
    fn parse_gsp_result_redirect_default_destination() {
        let json = r#"{"redirect":{}}"#;
        assert_eq!(
            parse_gsp_result(json),
            GspOutcome::Redirect {
                destination: "/".to_string()
            }
        );
    }

    #[test]
    fn parse_gsp_result_not_found() {
        let json = r#"{"notFound":true}"#;
        assert_eq!(parse_gsp_result(json), GspOutcome::NotFound);
    }

    #[test]
    fn parse_gsp_result_not_found_false_returns_empty_props() {
        let json = r#"{"notFound":false}"#;
        assert_eq!(parse_gsp_result(json), GspOutcome::Props("{}".to_string()));
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
