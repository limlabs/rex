use anyhow::Result;
use rex_build::build_bundles;
use rex_core::{AssetManifest, ProjectConfig, RexConfig};
use rex_router::scan_project;
use std::path::PathBuf;
use tracing::debug;

use crate::display::*;

pub(crate) async fn cmd_build(root: PathBuf) -> Result<()> {
    let root = std::fs::canonicalize(&root)?;
    let config = RexConfig::new(root);
    config.validate()?;

    print_mascot_header(env!("CARGO_PKG_VERSION"), "");

    let start = std::time::Instant::now();
    debug!("Building for production...");

    let scan = scan_project(&config.project_root, &config.pages_dir, &config.app_dir)?;
    debug!(routes = scan.routes.len(), "Routes scanned");

    let project_config = ProjectConfig::load(&config.project_root)?;
    let build_result = build_bundles(&config, &scan, &project_config).await?;
    let elapsed = start.elapsed();

    eprintln!(
        "  {} {}",
        green_bold("✓ Built in"),
        green_bold(&format_duration(elapsed))
    );
    eprintln!();

    // Server bundle size
    let server_size = std::fs::metadata(&build_result.server_bundle_path)
        .map(|m| m.len())
        .unwrap_or(0);
    eprintln!("  {}  {}", dim("Server"), format_size(server_size));

    // Client bundle sizes
    let client_dir = config.client_build_dir();
    let mut total_client: u64 = 0;
    let mut page_sizes: Vec<(String, u64)> = Vec::new();
    let mut chunk_sizes: Vec<(String, u64)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&client_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "js") {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                total_client += size;
                if name.starts_with("chunk-") {
                    chunk_sizes.push((name, size));
                } else {
                    page_sizes.push((name, size));
                }
            }
        }
    }

    eprintln!("  {}  {}", dim("Client"), format_size(total_client));
    eprintln!();

    // Show page entry chunks
    page_sizes.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, size) in &page_sizes {
        eprintln!(
            "    {}  {}",
            dim(&format!("{:<38}", name)),
            dim(&format_size(*size))
        );
    }

    // Show shared chunks
    chunk_sizes.sort_by(|a, b| b.1.cmp(&a.1)); // largest first
    for (name, size) in &chunk_sizes {
        eprintln!(
            "    {}  {}",
            dim(&format!("{:<38}", name)),
            dim(&format_size(*size))
        );
    }

    eprintln!();
    print_route_summary_with_manifest(&scan.routes, &scan.api_routes, &build_result.manifest);
    eprintln!();
    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// Classified route info for build summary display.
struct RouteInfo {
    pattern: String,
    icon: &'static str,
    label: &'static str,
}

/// Classify routes by render mode and count static vs. server-rendered pages.
///
/// Returns (route_infos, static_count, ssg_count, dynamic_count) sorted by pattern.
fn classify_routes(
    routes: &[rex_core::Route],
    manifest: &AssetManifest,
) -> (Vec<RouteInfo>, usize, usize, usize) {
    use rex_core::RenderMode;

    let mut sorted: Vec<_> = routes.iter().collect();
    sorted.sort_by(|a, b| a.pattern.cmp(&b.pattern));

    let mut static_count = 0usize;
    let mut ssg_count = 0usize;
    let mut dynamic_count = 0usize;
    let mut infos = Vec::with_capacity(sorted.len());

    for route in &sorted {
        let page_assets = manifest.pages.get(&route.pattern);

        let render_mode = page_assets
            .map(|p| p.render_mode)
            .unwrap_or(RenderMode::ServerRendered);

        let has_static_paths = page_assets.is_some_and(|p| p.has_static_paths);

        let (icon, label) = if has_static_paths {
            ssg_count += 1;
            ("\u{25cf}", "SSG") // ●
        } else {
            match render_mode {
                RenderMode::Static => {
                    static_count += 1;
                    ("\u{25cb}", "static") // ○
                }
                RenderMode::ServerRendered => {
                    dynamic_count += 1;
                    ("\u{03bb}", "server") // λ
                }
            }
        };

        infos.push(RouteInfo {
            pattern: route.pattern.clone(),
            icon,
            label,
        });
    }

    (infos, static_count, ssg_count, dynamic_count)
}

/// Classify app routes into static/server-rendered categories.
fn classify_app_routes(manifest: &AssetManifest) -> (Vec<RouteInfo>, usize, usize) {
    use rex_core::RenderMode;

    let mut sorted: Vec<_> = manifest.app_routes.keys().collect();
    sorted.sort();

    let mut static_count = 0usize;
    let mut dynamic_count = 0usize;
    let mut infos = Vec::with_capacity(sorted.len());

    for pattern in &sorted {
        let render_mode = manifest
            .app_routes
            .get(*pattern)
            .map(|a| a.render_mode)
            .unwrap_or(RenderMode::ServerRendered);

        let (icon, label) = match render_mode {
            RenderMode::Static => {
                static_count += 1;
                ("\u{25cb}", "static") // ○
            }
            RenderMode::ServerRendered => {
                dynamic_count += 1;
                ("\u{03bb}", "server") // λ
            }
        };

        infos.push(RouteInfo {
            pattern: (*pattern).clone(),
            icon,
            label,
        });
    }

    (infos, static_count, dynamic_count)
}

fn print_route_summary_with_manifest(
    routes: &[rex_core::Route],
    api_routes: &[rex_core::Route],
    manifest: &AssetManifest,
) {
    if routes.is_empty() && api_routes.is_empty() && manifest.app_routes.is_empty() {
        return;
    }

    let (infos, mut static_count, ssg_count, mut dynamic_count) = classify_routes(routes, manifest);

    // Pages router routes
    for info in &infos {
        eprintln!(
            "    {} {} {}",
            dim(info.icon),
            dim(&format!("{:<30}", info.pattern)),
            dim(&format!("({})", info.label))
        );
    }

    // App router routes
    if !manifest.app_routes.is_empty() {
        let (app_infos, app_static, app_dynamic) = classify_app_routes(manifest);
        static_count += app_static;
        dynamic_count += app_dynamic;

        for info in &app_infos {
            eprintln!(
                "    {} {} {}",
                dim(info.icon),
                dim(&format!("{:<30}", info.pattern)),
                dim(&format!("({})", info.label))
            );
        }
    }

    for route in api_routes {
        eprintln!(
            "    {} {} {}",
            dim("\u{03bb}"),
            dim(&format!("{:<30}", route.pattern)),
            dim("(api)")
        );
    }

    eprintln!();

    let mut legend = Vec::new();
    if static_count > 0 {
        legend.push(format!("\u{25cb} static ({static_count})"));
    }
    if ssg_count > 0 {
        legend.push(format!("\u{25cf} SSG ({ssg_count})"));
    }
    if dynamic_count > 0 {
        legend.push(format!("\u{03bb} server ({dynamic_count})"));
    }
    if !api_routes.is_empty() {
        legend.push(format!("\u{03bb} api ({})", api_routes.len()));
    }
    if !legend.is_empty() {
        eprintln!("  {}", dim(&legend.join("  ")));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::AssetManifest;

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024 - 1), "1024.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }

    fn make_route(
        pattern: &str,
        dynamic_segments: Vec<rex_core::DynamicSegment>,
    ) -> rex_core::Route {
        rex_core::Route {
            pattern: pattern.into(),
            file_path: format!("pages{pattern}.tsx").into(),
            abs_path: format!("/pages{pattern}.tsx").into(),
            page_type: rex_core::PageType::Regular,
            dynamic_segments,
            specificity: 0,
        }
    }

    #[test]
    fn classify_routes_static_and_dynamic() {
        use rex_core::DataStrategy;

        let routes = vec![
            make_route("/about", vec![]),
            make_route("/", vec![]),
            make_route(
                "/blog/:slug",
                vec![rex_core::DynamicSegment::Single("slug".into())],
            ),
        ];

        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/", "index.js", DataStrategy::None, false);
        manifest.add_page("/about", "about.js", DataStrategy::None, false);
        manifest.add_page("/blog/:slug", "slug.js", DataStrategy::None, true);

        let (infos, static_count, _ssg_count, dynamic_count) = classify_routes(&routes, &manifest);

        // Sorted by pattern
        assert_eq!(infos[0].pattern, "/");
        assert_eq!(infos[1].pattern, "/about");
        assert_eq!(infos[2].pattern, "/blog/:slug");

        assert_eq!(static_count, 2);
        assert_eq!(dynamic_count, 1);

        assert_eq!(infos[0].label, "static");
        assert_eq!(infos[2].label, "server");
    }

    #[test]
    fn classify_routes_gssp_is_server() {
        use rex_core::DataStrategy;

        let routes = vec![make_route("/dashboard", vec![])];

        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page(
            "/dashboard",
            "dashboard.js",
            DataStrategy::GetServerSideProps,
            false,
        );

        let (infos, static_count, _ssg_count, dynamic_count) = classify_routes(&routes, &manifest);

        assert_eq!(static_count, 0);
        assert_eq!(dynamic_count, 1);
        assert_eq!(infos[0].label, "server");
    }

    #[test]
    fn classify_routes_gsp_is_static() {
        use rex_core::DataStrategy;

        let routes = vec![make_route("/posts", vec![])];

        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/posts", "posts.js", DataStrategy::GetStaticProps, false);

        let (_, static_count, _ssg_count, dynamic_count) = classify_routes(&routes, &manifest);

        assert_eq!(static_count, 1);
        assert_eq!(dynamic_count, 0);
    }

    #[test]
    fn classify_routes_static_paths_is_ssg() {
        use rex_core::DataStrategy;

        let routes = vec![make_route(
            "/posts/:id",
            vec![rex_core::DynamicSegment::Single("id".into())],
        )];

        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/posts/:id", "posts.js", DataStrategy::GetStaticProps, true);
        if let Some(page) = manifest.pages.get_mut("/posts/:id") {
            page.has_static_paths = true;
        }

        let (infos, static_count, ssg_count, dynamic_count) = classify_routes(&routes, &manifest);

        assert_eq!(static_count, 0);
        assert_eq!(ssg_count, 1);
        assert_eq!(dynamic_count, 0);
        assert_eq!(infos[0].label, "SSG");
        assert_eq!(infos[0].icon, "\u{25cf}");
    }

    #[test]
    fn classify_app_routes_static_and_dynamic() {
        use rex_core::{AppRouteAssets, RenderMode};

        let mut manifest = AssetManifest::new("test".into());
        manifest.app_routes.insert(
            "/".to_string(),
            AppRouteAssets {
                client_chunks: vec![],
                layout_chain: vec![],
                render_mode: RenderMode::Static,
            },
        );
        manifest.app_routes.insert(
            "/about".to_string(),
            AppRouteAssets {
                client_chunks: vec![],
                layout_chain: vec![],
                render_mode: RenderMode::Static,
            },
        );
        manifest.app_routes.insert(
            "/blog/:slug".to_string(),
            AppRouteAssets {
                client_chunks: vec![],
                layout_chain: vec![],
                render_mode: RenderMode::ServerRendered,
            },
        );

        let (infos, static_count, dynamic_count) = classify_app_routes(&manifest);

        assert_eq!(static_count, 2);
        assert_eq!(dynamic_count, 1);
        assert_eq!(infos.len(), 3);

        // Verify sorted order
        assert_eq!(infos[0].pattern, "/");
        assert_eq!(infos[1].pattern, "/about");
        assert_eq!(infos[2].pattern, "/blog/:slug");
    }

    #[test]
    fn classify_app_routes_empty() {
        let manifest = AssetManifest::new("test".into());
        let (infos, static_count, dynamic_count) = classify_app_routes(&manifest);
        assert!(infos.is_empty());
        assert_eq!(static_count, 0);
        assert_eq!(dynamic_count, 0);
    }
}
