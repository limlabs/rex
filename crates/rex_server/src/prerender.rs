use crate::document::{
    assemble_body_tail, assemble_head_shell, assemble_rsc_document, extract_body_tag_attrs,
    extract_html_tag_attrs, DocumentDescriptor, RscDocumentParams,
};
use rex_core::{AppRouteAssets, AssetManifest, Fallback, PageAssets, RenderMode, Route};
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

/// Pre-render pages that export `getStaticPaths` — call getStaticPaths to discover
/// concrete URL paths, then call getStaticProps + render for each one.
///
/// Returns a map of concrete URL path -> PrerenderedPage.
/// Also mutates the manifest to store the resolved fallback for each page.
pub async fn prerender_static_path_pages(
    pool: &IsolatePool,
    manifest: &mut AssetManifest,
    routes: &[Route],
    manifest_json: &str,
    doc_descriptor: Option<&DocumentDescriptor>,
) -> HashMap<String, PrerenderedPage> {
    let mut prerendered = HashMap::new();

    let route_by_pattern: HashMap<&str, &Route> =
        routes.iter().map(|r| (r.pattern.as_str(), r)).collect();

    // Collect pages that have getStaticPaths
    let static_path_pages: Vec<(String, PageAssets)> = manifest
        .pages
        .iter()
        .filter(|(_, assets)| assets.has_static_paths)
        .map(|(pattern, assets)| (pattern.clone(), assets.clone()))
        .collect();

    for (pattern, assets) in &static_path_pages {
        let route = match route_by_pattern.get(pattern.as_str()) {
            Some(r) => *r,
            None => {
                warn!(pattern, "No route found for static-path page, skipping");
                continue;
            }
        };
        let route_key = route.module_name();

        // Call getStaticPaths to get the list of paths + fallback
        let key = route_key.clone();
        let paths_json = match pool.execute(move |iso| iso.get_static_paths(&key)).await {
            Ok(Ok(json)) => json,
            Ok(Err(e)) => {
                warn!(pattern, error = %e, "getStaticPaths failed, skipping");
                continue;
            }
            Err(e) => {
                warn!(pattern, error = %e, "Pool error during getStaticPaths");
                continue;
            }
        };

        let (param_sets, fallback) = match parse_static_paths_result(&paths_json) {
            Some(r) => r,
            None => {
                warn!(pattern, "Failed to parse getStaticPaths result, skipping");
                continue;
            }
        };

        // Store fallback in manifest
        if let Some(page) = manifest.pages.get_mut(pattern) {
            page.fallback = fallback;
        }

        debug!(
            pattern,
            paths = param_sets.len(),
            ?fallback,
            "getStaticPaths resolved"
        );

        // For each set of params, build the concrete URL path, call GSP, and render
        for params in &param_sets {
            let concrete_path = params_to_path(pattern, params);

            // Call getStaticProps with these params
            let key = route_key.clone();
            let ctx = serde_json::json!({ "params": params }).to_string();
            let props_json = match pool
                .execute(move |iso| iso.get_static_props(&key, &ctx))
                .await
            {
                Ok(Ok(json)) => match parse_gsp_result(&json) {
                    GspOutcome::Props(props) => props,
                    GspOutcome::Redirect { destination } => {
                        debug!(
                            concrete_path,
                            destination, "getStaticProps returned redirect, skipping"
                        );
                        continue;
                    }
                    GspOutcome::NotFound => {
                        debug!(concrete_path, "getStaticProps returned notFound, skipping");
                        continue;
                    }
                },
                Ok(Err(e)) => {
                    warn!(concrete_path, error = %e, "getStaticProps failed, skipping");
                    continue;
                }
                Err(e) => {
                    warn!(concrete_path, error = %e, "Pool error during getStaticProps");
                    continue;
                }
            };

            // Render the page via V8
            let key = route_key.clone();
            let props = props_json.clone();
            let render_result = match pool.execute(move |iso| iso.render_page(&key, &props)).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    warn!(concrete_path, error = %e, "SSR render failed, skipping");
                    continue;
                }
                Err(e) => {
                    warn!(concrete_path, error = %e, "Pool error during render");
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
            debug!(
                concrete_path,
                bytes = html.len(),
                "Pre-rendered static path page"
            );
            prerendered.insert(concrete_path, PrerenderedPage { html, props_json });
        }
    }

    if !prerendered.is_empty() {
        info!(
            count = prerendered.len(),
            "Static path pages pre-rendered (getStaticPaths)"
        );
    }

    prerendered
}

/// Parse the JSON result from getStaticPaths.
///
/// Expected shape: `{ paths: [{ params: { id: "1" } }, ...], fallback: false | "blocking" }`
pub(crate) fn parse_static_paths_result(
    json: &str,
) -> Option<(Vec<HashMap<String, serde_json::Value>>, Fallback)> {
    let val: serde_json::Value = serde_json::from_str(json).ok()?;

    let paths = val.get("paths")?.as_array()?;
    let mut param_sets = Vec::new();
    for path_entry in paths {
        let params_val = path_entry.get("params")?;
        let params: HashMap<String, serde_json::Value> =
            serde_json::from_value(params_val.clone()).ok()?;
        param_sets.push(params);
    }

    let fallback = match val.get("fallback") {
        Some(serde_json::Value::Bool(false)) | None => Fallback::False,
        Some(serde_json::Value::String(s)) if s == "blocking" => Fallback::Blocking,
        Some(serde_json::Value::Bool(true)) => {
            warn!("fallback: true is not yet supported; treating as blocking");
            Fallback::Blocking
        }
        _ => Fallback::False,
    };

    Some((param_sets, fallback))
}

/// Convert a route pattern + params to a concrete URL path.
///
/// Example: pattern = "/posts/:id", params = {"id": "first"} -> "/posts/first"
pub(crate) fn params_to_path(pattern: &str, params: &HashMap<String, serde_json::Value>) -> String {
    let mut path = pattern.to_string();
    for (key, value) in params {
        let value_str = match value {
            serde_json::Value::String(s) => s.clone(),
            // Array params for catch-all routes: ["2020", "01", "01"] -> "2020/01/01"
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("/"),
            serde_json::Value::Number(n) => n.to_string(),
            _ => value.to_string(),
        };
        // Handle both :param and :param* (catch-all) patterns
        path = path
            .replace(&format!(":{key}*"), &value_str)
            .replace(&format!(":{key}"), &value_str);
    }
    path
}

#[cfg(test)]
#[path = "prerender_tests.rs"]
mod tests;
