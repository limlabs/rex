use crate::document::{
    assemble_body_tail, assemble_head_shell, assemble_rsc_document, extract_body_tag_attrs,
    extract_html_tag_attrs, DocumentDescriptor, RscDocumentParams,
};
use rex_core::{AppRouteAssets, AssetManifest, PageAssets, RenderMode, Route};
use rex_v8::IsolatePool;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Pre-rendered page: contains the full HTML and the page props JSON.
#[derive(Debug, Clone)]
pub struct PrerenderedPage {
    /// Full HTML document (for initial page load)
    pub html: String,
    /// Page props JSON (for client-side navigation data files)
    pub props_json: String,
}

/// Pre-render all statically optimized pages and return a map of route pattern -> result.
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
) -> HashMap<String, PrerenderedPage> {
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
        prerendered.insert(pattern.clone(), PrerenderedPage { html, props_json });
    }

    if !prerendered.is_empty() {
        info!(
            count = prerendered.len(),
            "Static pages pre-rendered (automatic static optimization)"
        );
    }

    prerendered
}

/// Pre-rendered app route: contains both the full HTML (for initial page loads)
/// and the flight data (for client-side RSC navigations).
#[derive(Debug, Clone)]
pub struct PrerenderedAppRoute {
    /// Full HTML document (for initial page load)
    pub html: String,
    /// RSC flight data (for client-side navigation via /_rex/rsc/ endpoint)
    pub flight: String,
}

/// Pre-render all statically optimized app routes and return a map of route pattern -> result.
///
/// App routes are eligible for static optimization when their `render_mode` is `Static`:
/// - No dynamic route segments (e.g., `[slug]`)
/// - No dynamic function usage (`cookies()`, `headers()`) in the component tree
pub async fn prerender_static_app_routes(
    pool: &IsolatePool,
    manifest: &AssetManifest,
    manifest_json: &str,
) -> HashMap<String, PrerenderedAppRoute> {
    let mut prerendered = HashMap::new();

    let static_app_routes = collect_static_app_routes(manifest);

    // Client manifest JSON needed for RSC document assembly
    let client_manifest_json = manifest
        .client_reference_manifest
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok())
        .unwrap_or_else(|| "{}".to_string());

    for (pattern, assets) in &static_app_routes {
        // App routes use the pattern as the route key (registered in V8 by pattern)
        let route_key = pattern.clone();

        // Static app routes have no params and no searchParams
        let props_json = serde_json::json!({ "params": {}, "searchParams": {} }).to_string();

        // Single render pass: render_rsc_to_html returns HTML + flight data together
        let rsc_result = match pool
            .execute(move |iso| iso.render_rsc_to_html(&route_key, &props_json))
            .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                warn!(pattern, error = %e, "RSC render failed, skipping pre-render");
                continue;
            }
            Err(e) => {
                warn!(pattern, error = %e, "Pool error during RSC render");
                continue;
            }
        };

        // Extract <html> and <body> attributes from the SSR output so the
        // served HTML matches the RSC flight data (prevents hydration mismatch).
        let html_attrs = extract_html_tag_attrs(&rsc_result.body).to_string();
        let body_attrs = extract_body_tag_attrs(&rsc_result.body).to_string();

        let html = assemble_rsc_document(&RscDocumentParams {
            ssr_html: &rsc_result.body,
            head_html: &rsc_result.head,
            flight_data: &rsc_result.flight,
            client_chunks: &assets.client_chunks,
            client_manifest_json: &client_manifest_json,
            css_files: &manifest.global_css,
            css_contents: &manifest.css_contents,
            is_dev: false,
            manifest_json: Some(manifest_json),
            html_attrs: &html_attrs,
            body_attrs: &body_attrs,
        });

        debug!(
            pattern,
            html_bytes = html.len(),
            flight_bytes = rsc_result.flight.len(),
            "Pre-rendered static app route"
        );
        prerendered.insert(
            pattern.clone(),
            PrerenderedAppRoute {
                html,
                flight: rsc_result.flight,
            },
        );
    }

    if !prerendered.is_empty() {
        info!(
            count = prerendered.len(),
            "Static app routes pre-rendered (automatic static optimization)"
        );
    }

    prerendered
}

/// Collect app routes eligible for static pre-rendering from the manifest.
fn collect_static_app_routes(manifest: &AssetManifest) -> Vec<(String, AppRouteAssets)> {
    manifest
        .app_routes
        .iter()
        .filter(|(_, assets)| assets.render_mode == RenderMode::Static)
        .map(|(pattern, assets)| (pattern.clone(), assets.clone()))
        .collect()
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
        None,
    );

    let tail = assemble_body_tail(
        &render_result.body,
        &render_result.head,
        props_json,
        &client_scripts,
        manifest.app_script.as_deref(),
        false, // never dev mode for pre-rendered pages
        Some(manifest_json),
        false,
    );

    format!("{shell}{tail}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::{AppRouteAssets, DataStrategy};

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

    #[test]
    fn collect_static_app_routes_filters_correctly() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.app_routes.insert(
            "/".to_string(),
            AppRouteAssets {
                client_chunks: vec!["chunk.js".into()],
                layout_chain: vec![],
                render_mode: RenderMode::Static,
            },
        );
        manifest.app_routes.insert(
            "/about".to_string(),
            AppRouteAssets {
                client_chunks: vec!["chunk.js".into()],
                layout_chain: vec![],
                render_mode: RenderMode::Static,
            },
        );
        manifest.app_routes.insert(
            "/blog/:slug".to_string(),
            AppRouteAssets {
                client_chunks: vec!["chunk.js".into()],
                layout_chain: vec![],
                render_mode: RenderMode::ServerRendered,
            },
        );
        manifest.app_routes.insert(
            "/dashboard".to_string(),
            AppRouteAssets {
                client_chunks: vec!["chunk.js".into()],
                layout_chain: vec![],
                render_mode: RenderMode::ServerRendered,
            },
        );

        let static_routes = collect_static_app_routes(&manifest);
        let patterns: Vec<&str> = static_routes.iter().map(|(p, _)| p.as_str()).collect();

        assert_eq!(static_routes.len(), 2);
        assert!(patterns.contains(&"/"));
        assert!(patterns.contains(&"/about"));
    }

    #[test]
    fn collect_static_app_routes_empty_manifest() {
        let manifest = AssetManifest::new("test".into());
        assert!(collect_static_app_routes(&manifest).is_empty());
    }

    #[test]
    fn collect_static_app_routes_all_server_rendered() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.app_routes.insert(
            "/".to_string(),
            AppRouteAssets {
                client_chunks: vec![],
                layout_chain: vec![],
                render_mode: RenderMode::ServerRendered,
            },
        );
        assert!(collect_static_app_routes(&manifest).is_empty());
    }
}
