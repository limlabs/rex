#![allow(clippy::unwrap_used)]

use rex_core::DynamicSegment;
use rex_router::scan_app;
use std::fs;
use std::path::Path;

fn setup_fixture(dir: &Path) {
    // app/
    //   layout.tsx
    //   page.tsx
    //   loading.tsx
    //   about/
    //     page.tsx
    //   blog/
    //     page.tsx
    //     [slug]/
    //       page.tsx
    //       layout.tsx
    //   (marketing)/
    //     pricing/
    //       page.tsx
    //   dashboard/
    //     layout.tsx
    //     page.tsx
    //     settings/
    //       page.tsx

    let dirs = [
        "",
        "about",
        "blog",
        "blog/[slug]",
        "(marketing)",
        "(marketing)/pricing",
        "dashboard",
        "dashboard/settings",
    ];
    for d in &dirs {
        fs::create_dir_all(dir.join(d)).unwrap();
    }

    let files = [
        "layout.tsx",
        "page.tsx",
        "loading.tsx",
        "about/page.tsx",
        "blog/page.tsx",
        "blog/[slug]/page.tsx",
        "blog/[slug]/layout.tsx",
        "(marketing)/pricing/page.tsx",
        "dashboard/layout.tsx",
        "dashboard/page.tsx",
        "dashboard/settings/page.tsx",
    ];
    for f in &files {
        fs::write(dir.join(f), format!("// {f}")).unwrap();
    }
}

#[test]
fn scans_app_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();

    assert!(result.root_layout.as_ref().unwrap().exists());
    assert!(!result.routes.is_empty());
}

#[test]
fn correct_route_count() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();

    // Routes: /, /about, /blog, /blog/:slug, /pricing, /dashboard, /dashboard/settings
    assert_eq!(result.routes.len(), 7);
}

#[test]
fn route_group_excluded_from_url() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();
    let patterns: Vec<&str> = result.routes.iter().map(|r| r.pattern.as_str()).collect();

    // (marketing)/pricing → /pricing (group not in URL)
    assert!(patterns.contains(&"/pricing"));
    assert!(!patterns.iter().any(|p| p.contains("marketing")));
}

#[test]
fn dynamic_segment_parsed() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();
    let blog_slug = result
        .routes
        .iter()
        .find(|r| r.pattern == "/blog/:slug")
        .expect("should have /blog/:slug route");

    assert_eq!(blog_slug.dynamic_segments.len(), 1);
    assert!(matches!(
        &blog_slug.dynamic_segments[0],
        DynamicSegment::Single(n) if n == "slug"
    ));
}

#[test]
fn layout_chain_ordering() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();

    // /blog/:slug has two layouts: root + blog/[slug]/layout.tsx
    let blog_slug = result
        .routes
        .iter()
        .find(|r| r.pattern == "/blog/:slug")
        .unwrap();
    assert_eq!(blog_slug.layout_chain.len(), 2);
    // First should be root layout
    assert!(blog_slug.layout_chain[0]
        .to_string_lossy()
        .ends_with("layout.tsx"));
    // Second should be blog/[slug]/layout.tsx
    assert!(blog_slug.layout_chain[1]
        .to_string_lossy()
        .contains("[slug]"));

    // /about has only root layout
    let about = result
        .routes
        .iter()
        .find(|r| r.pattern == "/about")
        .unwrap();
    assert_eq!(about.layout_chain.len(), 1);
}

#[test]
fn nested_layout_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();

    // /dashboard/settings has root layout + dashboard/layout.tsx
    let settings = result
        .routes
        .iter()
        .find(|r| r.pattern == "/dashboard/settings")
        .unwrap();
    assert_eq!(settings.layout_chain.len(), 2);
}

#[test]
fn loading_chain_parallel() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();

    // Root has loading.tsx, so first entry is Some
    let root_route = result.routes.iter().find(|r| r.pattern == "/").unwrap();
    assert_eq!(root_route.loading_chain.len(), 1);
    assert!(root_route.loading_chain[0].is_some());
}

#[test]
fn no_app_dir_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let result = scan_app(&tmp.path().join("nonexistent")).unwrap();
    assert!(result.is_none());
}

#[test]
fn no_root_layout_no_groups_returns_none() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(app_dir.join("page.tsx"), "// page").unwrap();
    // No layout.tsx, no route groups

    let result = scan_app(&app_dir).unwrap();
    assert!(result.is_none());
}

#[test]
fn route_groups_with_own_layouts_no_root_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");

    // app/
    //   (frontend)/
    //     layout.tsx     <- group provides its own layout
    //     page.tsx
    //     about/
    //       page.tsx
    //   (admin)/
    //     layout.tsx
    //     dashboard/
    //       page.tsx
    fs::create_dir_all(app_dir.join("(frontend)/about")).unwrap();
    fs::create_dir_all(app_dir.join("(admin)/dashboard")).unwrap();

    fs::write(app_dir.join("(frontend)/layout.tsx"), "// frontend layout").unwrap();
    fs::write(app_dir.join("(frontend)/page.tsx"), "// home page").unwrap();
    fs::write(app_dir.join("(frontend)/about/page.tsx"), "// about page").unwrap();
    fs::write(app_dir.join("(admin)/layout.tsx"), "// admin layout").unwrap();
    fs::write(app_dir.join("(admin)/dashboard/page.tsx"), "// dashboard").unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    // No root layout
    assert!(result.root_layout.is_none());

    // Should have 3 routes: /, /about, /dashboard
    assert_eq!(result.routes.len(), 3);

    let patterns: Vec<&str> = result.routes.iter().map(|r| r.pattern.as_str()).collect();
    assert!(patterns.contains(&"/"));
    assert!(patterns.contains(&"/about"));
    assert!(patterns.contains(&"/dashboard"));

    // Each route's layout chain should contain its group's layout
    let home = result.routes.iter().find(|r| r.pattern == "/").unwrap();
    assert_eq!(home.layout_chain.len(), 1);
    assert!(home.layout_chain[0]
        .to_string_lossy()
        .contains("(frontend)"));

    let dashboard = result
        .routes
        .iter()
        .find(|r| r.pattern == "/dashboard")
        .unwrap();
    assert_eq!(dashboard.layout_chain.len(), 1);
    assert!(dashboard.layout_chain[0]
        .to_string_lossy()
        .contains("(admin)"));
}

#[test]
fn error_boundary_in_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("protected")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(app_dir.join("page.tsx"), "// root page").unwrap();
    // Error/loading chains are parallel to layout chain — a layout is needed
    // at the protected segment for the error boundary to appear in the chain.
    fs::write(app_dir.join("protected/layout.tsx"), "// protected layout").unwrap();
    fs::write(app_dir.join("protected/page.tsx"), "// protected page").unwrap();
    fs::write(app_dir.join("protected/error.tsx"), "// error boundary").unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();
    let protected = result
        .routes
        .iter()
        .find(|r| r.pattern == "/protected")
        .expect("should have /protected route");

    // Error chain should have entries parallel to layout chain
    assert_eq!(protected.error_chain.len(), protected.layout_chain.len());
    // The protected segment has error.tsx + layout, so the second entry should be Some
    assert!(
        protected.error_chain.iter().any(|e| e.is_some()),
        "error chain should contain the error boundary"
    );
    // Verify the error boundary path
    let error_path = protected
        .error_chain
        .iter()
        .find_map(|e| e.as_ref())
        .unwrap();
    assert!(error_path.to_string_lossy().contains("error.tsx"));
}

#[test]
fn catch_all_route() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("docs/[...slug]")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(app_dir.join("page.tsx"), "// root page").unwrap();
    fs::write(app_dir.join("docs/[...slug]/page.tsx"), "// catch-all page").unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();
    let catch_all = result
        .routes
        .iter()
        .find(|r| {
            r.dynamic_segments
                .iter()
                .any(|d| matches!(d, DynamicSegment::CatchAll(_)))
        })
        .expect("should have a catch-all route");

    assert!(catch_all.pattern.contains("*slug"));
    assert_eq!(catch_all.dynamic_segments.len(), 1);
    assert!(matches!(
        &catch_all.dynamic_segments[0],
        DynamicSegment::CatchAll(n) if n == "slug"
    ));
}

#[test]
fn optional_catch_all_route() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("help/[[...path]]")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(app_dir.join("page.tsx"), "// root page").unwrap();
    fs::write(
        app_dir.join("help/[[...path]]/page.tsx"),
        "// optional catch-all",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();
    let opt_catch_all = result
        .routes
        .iter()
        .find(|r| {
            r.dynamic_segments
                .iter()
                .any(|d| matches!(d, DynamicSegment::OptionalCatchAll(_)))
        })
        .expect("should have an optional catch-all route");

    assert_eq!(opt_catch_all.dynamic_segments.len(), 1);
    assert!(matches!(
        &opt_catch_all.dynamic_segments[0],
        DynamicSegment::OptionalCatchAll(n) if n == "path"
    ));
}

#[test]
fn specificity_ordering() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    setup_fixture(&app_dir);

    let result = scan_app(&app_dir).unwrap().unwrap();

    // Routes should be sorted by specificity descending.
    // Static routes have higher specificity than dynamic ones.
    for window in result.routes.windows(2) {
        assert!(window[0].specificity >= window[1].specificity);
    }
}

// --- route.ts tests ---

#[test]
fn route_handler_basic() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("api/hello")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(
        app_dir.join("api/hello/route.ts"),
        "export function GET() {}",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    assert_eq!(result.api_routes.len(), 1);
    assert_eq!(result.api_routes[0].pattern, "/api/hello");
    assert!(result.api_routes[0].dynamic_segments.is_empty());
    assert!(result.api_routes[0]
        .handler_path
        .to_string_lossy()
        .ends_with("route.ts"));
}

#[test]
fn route_handler_dynamic_segment() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("api/users/[id]")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(
        app_dir.join("api/users/[id]/route.ts"),
        "export function GET() {}",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    assert_eq!(result.api_routes.len(), 1);
    assert_eq!(result.api_routes[0].pattern, "/api/users/:id");
    assert_eq!(result.api_routes[0].dynamic_segments.len(), 1);
    assert!(matches!(
        &result.api_routes[0].dynamic_segments[0],
        DynamicSegment::Single(n) if n == "id"
    ));
}

#[test]
fn route_handler_in_route_group() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("(api)/webhooks")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(
        app_dir.join("(api)/webhooks/route.ts"),
        "export function POST() {}",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    assert_eq!(result.api_routes.len(), 1);
    // Route group should be excluded from the URL pattern
    assert_eq!(result.api_routes[0].pattern, "/webhooks");
}

#[test]
fn page_takes_priority_over_route() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("conflict")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(app_dir.join("conflict/page.tsx"), "// page component").unwrap();
    fs::write(
        app_dir.join("conflict/route.ts"),
        "export function GET() {}",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    // page.tsx wins — no api routes, but the page route exists
    assert!(result.api_routes.is_empty());
    assert!(result.routes.iter().any(|r| r.pattern == "/conflict"));
}

#[test]
fn api_only_app_dir_no_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("api/health")).unwrap();

    // No layout.tsx — only route handlers
    fs::write(
        app_dir.join("api/health/route.ts"),
        "export function GET() {}",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    // Should still produce a valid result with api routes
    assert_eq!(result.api_routes.len(), 1);
    assert_eq!(result.api_routes[0].pattern, "/api/health");
    // No page routes
    assert!(result.routes.is_empty());
}

#[test]
fn route_handler_at_root() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(&app_dir).unwrap();

    // route.ts at the root of app/ (no layout needed for API routes)
    fs::write(app_dir.join("route.ts"), "export function GET() {}").unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    assert_eq!(result.api_routes.len(), 1);
    assert_eq!(result.api_routes[0].pattern, "/");
}

#[test]
fn multiple_route_handlers_sorted_by_specificity() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("api/users/[id]")).unwrap();
    fs::create_dir_all(app_dir.join("api/health")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(
        app_dir.join("api/users/[id]/route.ts"),
        "export function GET() {}",
    )
    .unwrap();
    fs::write(
        app_dir.join("api/health/route.ts"),
        "export function GET() {}",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();

    assert_eq!(result.api_routes.len(), 2);
    // Static route (/api/health) should have higher specificity than dynamic (/api/users/:id)
    assert!(result.api_routes[0].specificity >= result.api_routes[1].specificity);
    let patterns: Vec<&str> = result
        .api_routes
        .iter()
        .map(|r| r.pattern.as_str())
        .collect();
    assert!(patterns.contains(&"/api/health"));
    assert!(patterns.contains(&"/api/users/:id"));
}

#[test]
fn to_api_routes_converts_correctly() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("api/test")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(
        app_dir.join("api/test/route.ts"),
        "export function GET() {}",
    )
    .unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();
    let core_routes = result.to_api_routes();

    assert_eq!(core_routes.len(), 1);
    assert_eq!(core_routes[0].pattern, "/api/test");
    assert_eq!(core_routes[0].page_type, rex_core::PageType::AppApi);
}

#[test]
fn to_routes_converts_page_routes() {
    let tmp = tempfile::tempdir().unwrap();
    let app_dir = tmp.path().join("app");
    fs::create_dir_all(app_dir.join("about")).unwrap();

    fs::write(app_dir.join("layout.tsx"), "// root layout").unwrap();
    fs::write(app_dir.join("page.tsx"), "// home page").unwrap();
    fs::write(app_dir.join("about/page.tsx"), "// about page").unwrap();

    let result = scan_app(&app_dir).unwrap().unwrap();
    assert_eq!(result.routes.len(), 2);

    let core_routes = result.to_routes();
    assert_eq!(core_routes.len(), 2);

    let patterns: Vec<&str> = core_routes.iter().map(|r| r.pattern.as_str()).collect();
    assert!(patterns.contains(&"/"));
    assert!(patterns.contains(&"/about"));
    for r in &core_routes {
        assert_eq!(r.page_type, rex_core::PageType::Regular);
    }
}
