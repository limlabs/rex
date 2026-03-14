#![allow(clippy::unwrap_used)]

use super::*;
use rex_core::{AppRouteAssets, DataStrategy, Fallback};

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

#[test]
fn parse_static_paths_basic() {
    let json =
        r#"{"paths":[{"params":{"id":"first"}},{"params":{"id":"second"}}],"fallback":false}"#;
    let (params, fallback) = parse_static_paths_result(json).unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(
        params[0].get("id").unwrap(),
        &serde_json::Value::String("first".to_string())
    );
    assert_eq!(fallback, Fallback::False);
}

#[test]
fn parse_static_paths_blocking() {
    let json = r#"{"paths":[{"params":{"slug":"hello"}}],"fallback":"blocking"}"#;
    let (params, fallback) = parse_static_paths_result(json).unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(fallback, Fallback::Blocking);
}

#[test]
fn parse_static_paths_empty() {
    let json = r#"{"paths":[],"fallback":false}"#;
    let (params, fallback) = parse_static_paths_result(json).unwrap();
    assert!(params.is_empty());
    assert_eq!(fallback, Fallback::False);
}

#[test]
fn parse_static_paths_invalid() {
    assert!(parse_static_paths_result("not json").is_none());
    assert!(parse_static_paths_result(r#"{"paths":"bad"}"#).is_none());
}

#[test]
fn params_to_path_single() {
    let mut params = HashMap::new();
    params.insert(
        "id".to_string(),
        serde_json::Value::String("first".to_string()),
    );
    assert_eq!(params_to_path("/posts/:id", &params), "/posts/first");
}

#[test]
fn params_to_path_multiple() {
    let mut params = HashMap::new();
    params.insert(
        "year".to_string(),
        serde_json::Value::String("2025".to_string()),
    );
    params.insert(
        "slug".to_string(),
        serde_json::Value::String("hello".to_string()),
    );
    assert_eq!(
        params_to_path("/blog/:year/:slug", &params),
        "/blog/2025/hello"
    );
}

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
fn params_to_path_catch_all_array() {
    let mut params = HashMap::new();
    params.insert("slug".to_string(), serde_json::json!(["2020", "01", "01"]));
    assert_eq!(
        params_to_path("/archive/:slug*", &params),
        "/archive/2020/01/01"
    );
}

#[test]
fn params_to_path_numeric() {
    let mut params = HashMap::new();
    params.insert("id".to_string(), serde_json::json!(42));
    assert_eq!(params_to_path("/items/:id", &params), "/items/42");
}

#[test]
fn parse_static_paths_fallback_true_treated_as_blocking() {
    let json = r#"{"paths":[{"params":{"id":"1"}}],"fallback":true}"#;
    let (params, fallback) = parse_static_paths_result(json).unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(fallback, Fallback::Blocking);
}

#[test]
fn parse_static_paths_unknown_fallback_defaults_to_false() {
    let json = r#"{"paths":[],"fallback":"unknown"}"#;
    let (_, fallback) = parse_static_paths_result(json).unwrap();
    assert_eq!(fallback, Fallback::False);
}

#[test]
fn params_to_path_catch_all_empty_array() {
    let mut params = HashMap::new();
    params.insert("slug".to_string(), serde_json::json!([]));
    assert_eq!(params_to_path("/docs/:slug*", &params), "/docs/");
}

#[test]
fn parse_static_paths_string_paths() {
    // Some users pass string paths instead of { params } objects — should fail gracefully
    let json = r#"{"paths":["/blog/a"],"fallback":false}"#;
    assert!(parse_static_paths_result(json).is_none());
}
