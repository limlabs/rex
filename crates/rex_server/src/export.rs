use crate::document::DocumentDescriptor;
use crate::prerender;
use rex_core::{AssetManifest, DataStrategy, RenderMode, Route};
use rex_v8::IsolatePool;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Configuration for the static export.
pub struct ExportConfig {
    /// Directory to write the exported site into.
    pub output_dir: PathBuf,
    /// Continue exporting even if some pages can't be statically rendered.
    pub force: bool,
    /// Base path prefix for asset URLs (e.g. "/rex" for GitHub Pages at user.github.io/rex/).
    pub base_path: String,
}

/// Result of a static export.
pub struct ExportResult {
    /// Number of pages successfully exported as HTML.
    pub pages_exported: usize,
    /// Pages that were skipped, with (route_pattern, reason).
    pub pages_skipped: Vec<(String, String)>,
}

/// Check which routes can be statically exported and return warnings/errors.
///
/// Returns `Err` if non-exportable routes exist and `force` is false.
/// Returns `Ok(warnings)` otherwise.
pub fn validate_exportability(
    manifest: &AssetManifest,
    force: bool,
) -> anyhow::Result<Vec<String>> {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    for (pattern, assets) in &manifest.pages {
        match assets.data_strategy {
            DataStrategy::GetServerSideProps => {
                let msg =
                    format!("Route {pattern} uses getServerSideProps (requires a running server)");
                if force {
                    warnings.push(msg);
                } else {
                    errors.push(msg);
                }
            }
            _ => {
                if assets.render_mode == RenderMode::ServerRendered {
                    let msg = format!("Route {pattern} has dynamic segments (cannot pre-render)");
                    if force {
                        warnings.push(msg);
                    } else {
                        errors.push(msg);
                    }
                }
            }
        }
    }

    for (pattern, assets) in &manifest.app_routes {
        if assets.render_mode == RenderMode::ServerRendered {
            let msg = format!("App route {pattern} is server-rendered (cannot pre-render)");
            if force {
                warnings.push(msg);
            } else {
                errors.push(msg);
            }
        }
    }

    if !errors.is_empty() {
        let summary = errors.join("\n  - ");
        anyhow::bail!(
            "Cannot export: some routes require a running server.\n  \
             - {summary}\n\n\
             Use --force to skip these routes and export only static pages."
        );
    }

    Ok(warnings)
}

/// Context needed for exporting a static site.
pub struct ExportContext<'a> {
    pub pool: &'a IsolatePool,
    pub manifest: &'a AssetManifest,
    pub routes: &'a [Route],
    pub manifest_json: &'a str,
    pub doc_descriptor: Option<&'a DocumentDescriptor>,
    pub client_build_dir: &'a Path,
    pub project_root: &'a Path,
}

/// Export all static-eligible pages to `config.output_dir` as a self-contained static site.
///
/// This is the core export engine. It:
/// 1. Pre-renders all static pages and app routes via V8
/// 2. Writes HTML files to the output directory
/// 3. Copies client assets (JS, CSS) to `_rex/static/`
/// 4. Writes `router.js` to `_rex/`
/// 5. Copies `public/` directory contents to the output root
pub async fn export_site(
    ctx: &ExportContext<'_>,
    config: &ExportConfig,
) -> anyhow::Result<ExportResult> {
    let output = &config.output_dir;
    let mut result = ExportResult {
        pages_exported: 0,
        pages_skipped: Vec::new(),
    };

    // Create output directory structure
    let static_dir = output.join("_rex").join("static");
    std::fs::create_dir_all(&static_dir)?;

    // 1. Pre-render pages router pages
    let prerendered_pages = prerender::prerender_static_pages(
        ctx.pool,
        ctx.manifest,
        ctx.routes,
        ctx.manifest_json,
        ctx.doc_descriptor,
    )
    .await;

    for (pattern, html) in &prerendered_pages {
        let file_path = route_to_file_path(output, pattern);
        let html = rewrite_asset_paths(html, &config.base_path);
        write_html_file(&file_path, &html)?;
        debug!(pattern, path = %file_path.display(), "Exported page");
        result.pages_exported += 1;
    }

    // Record skipped pages
    for (pattern, assets) in &ctx.manifest.pages {
        if prerendered_pages.contains_key(pattern) {
            continue;
        }
        let reason = match assets.data_strategy {
            DataStrategy::GetServerSideProps => "uses getServerSideProps".to_string(),
            _ if assets.render_mode == RenderMode::ServerRendered => {
                "dynamic route segments".to_string()
            }
            _ => "render failed".to_string(),
        };
        result.pages_skipped.push((pattern.clone(), reason));
    }

    // 2. Pre-render app router routes
    let prerendered_app =
        prerender::prerender_static_app_routes(ctx.pool, ctx.manifest, ctx.manifest_json).await;

    for (pattern, rendered) in &prerendered_app {
        let file_path = route_to_file_path(output, pattern);
        let html = rewrite_asset_paths(&rendered.html, &config.base_path);
        write_html_file(&file_path, &html)?;
        debug!(pattern, path = %file_path.display(), "Exported app route");
        result.pages_exported += 1;
    }

    // Record skipped app routes
    for pattern in ctx.manifest.app_routes.keys() {
        if prerendered_app.contains_key(pattern) {
            continue;
        }
        result
            .pages_skipped
            .push((pattern.clone(), "server-rendered".to_string()));
    }

    // 3. Export custom 404 page if it exists
    export_404_page(ctx, config, &mut result).await;

    // 4. Copy client assets from .rex/build/client/ → output/_rex/static/
    copy_client_assets(ctx.client_build_dir, &static_dir)?;

    // 5. Write router.js (with base_path rewritten)
    let router_js = include_str!(concat!(env!("OUT_DIR"), "/router.js"));
    let router_js = rewrite_asset_paths(router_js, &config.base_path);
    std::fs::write(output.join("_rex").join("router.js"), router_js)?;
    debug!("Wrote _rex/router.js");

    // 6. Copy public/ directory
    let public_dir = ctx.project_root.join("public");
    if public_dir.exists() {
        copy_dir_recursive(&public_dir, output)?;
        debug!(path = %public_dir.display(), "Copied public/ directory");
    }

    info!(
        exported = result.pages_exported,
        skipped = result.pages_skipped.len(),
        "Static export complete"
    );

    Ok(result)
}

/// Export the custom 404 page (if it exists) as `404.html` in the output root.
///
/// GitHub Pages and most static hosts serve `404.html` for missing routes.
async fn export_404_page(
    ctx: &ExportContext<'_>,
    config: &ExportConfig,
    result: &mut ExportResult,
) {
    // Check if a 404 page exists in the manifest
    let not_found_pattern = ctx.routes.iter().find(|r| r.pattern == "/404");
    if not_found_pattern.is_none() {
        return;
    }

    // The 404 page is typically already in prerendered pages (it's static).
    // If not, try to render it directly.
    let assets = match ctx.manifest.pages.get("/404") {
        Some(a) if a.render_mode == RenderMode::Static => a,
        _ => return,
    };

    let route = match not_found_pattern {
        Some(r) => r,
        None => return,
    };

    let route_key = route.module_name();
    let props_json = "{}".to_string();

    let key = route_key.clone();
    let props = props_json.clone();
    let render_result = match ctx
        .pool
        .execute(move |iso| iso.render_page(&key, &props))
        .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            warn!(error = %e, "Failed to render 404 page for export");
            return;
        }
        Err(e) => {
            warn!(error = %e, "Pool error rendering 404 page for export");
            return;
        }
    };

    let client_scripts: Vec<String> = vec![assets.js.clone()];
    let mut css_files = ctx.manifest.global_css.clone();
    css_files.extend(assets.css.iter().cloned());

    let shell = crate::document::assemble_head_shell(
        &css_files,
        &ctx.manifest.css_contents,
        &ctx.manifest.shared_chunks,
        ctx.manifest.app_script.as_deref(),
        &client_scripts,
        ctx.doc_descriptor,
        &ctx.manifest.font_preloads,
    );
    let tail = crate::document::assemble_body_tail(
        &render_result.body,
        &render_result.head,
        &props_json,
        &client_scripts,
        ctx.manifest.app_script.as_deref(),
        false,
        Some(ctx.manifest_json),
    );
    let html = rewrite_asset_paths(&format!("{shell}{tail}"), &config.base_path);

    let path = config.output_dir.join("404.html");
    if let Err(e) = std::fs::write(&path, &html) {
        warn!(error = %e, "Failed to write 404.html");
    } else {
        debug!("Exported 404.html");
        result.pages_exported += 1;
    }
}

/// Convert a route pattern like "/about" to a file path like "output/about.html".
fn route_to_file_path(output: &Path, pattern: &str) -> PathBuf {
    if pattern == "/" {
        output.join("index.html")
    } else {
        // "/about" -> "about.html", "/docs/intro" -> "docs/intro.html"
        let stripped = pattern.trim_start_matches('/');
        output.join(format!("{stripped}.html"))
    }
}

/// Write an HTML file, creating parent directories as needed.
fn write_html_file(path: &Path, html: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, html)?;
    Ok(())
}

/// Recursively copy all files from `client_build_dir` to `static_dir`.
fn copy_client_assets(client_build_dir: &Path, static_dir: &Path) -> anyhow::Result<()> {
    if !client_build_dir.exists() {
        return Ok(());
    }
    copy_dir_recursive(client_build_dir, static_dir)?;
    debug!(
        src = %client_build_dir.display(),
        dst = %static_dir.display(),
        "Copied client assets"
    );
    Ok(())
}

/// Rewrite asset paths and internal links in HTML/JS content to include the base path prefix.
///
/// When `base_path` is empty, returns the input unchanged.
/// When `base_path` is e.g. "/rex":
///   - `/_rex/` → `/rex/_rex/` (asset URLs)
///   - `href="/about"` → `href="/rex/about"` (navigation links)
///   - Injects `<script>window.__REX_BASE_PATH="/rex"</script>` in `<head>` so
///     client-side Link components can resolve paths after hydration.
fn rewrite_asset_paths(content: &str, base_path: &str) -> String {
    if base_path.is_empty() {
        return content.to_string();
    }
    // 1. Protect /_rex/ paths with a placeholder to avoid double-prefixing
    let content = content.replace("/_rex/", "\x00_REX_ASSET_\x00");
    // 2. Rewrite internal navigation links (href="/path")
    let content = content.replace("href=\"/", &format!("href=\"{base_path}/"));
    let content = content.replace("href='/", &format!("href='{base_path}/"));
    // 3. Restore /_rex/ paths with the base path prefix
    let content = content.replace("\x00_REX_ASSET_\x00", &format!("{base_path}/_rex/"));
    // 4. Inject base path global for client-side JS (Link component reads this after hydration)
    inject_base_path_global(&content, base_path)
}

/// Inject a `<script>` tag setting `window.__REX_BASE_PATH` for client-side code.
///
/// After React hydration, client components re-render with their original JS props,
/// overwriting the `href` attributes that `rewrite_asset_paths` fixed in static HTML.
/// The Link component reads `__REX_BASE_PATH` to prepend the correct prefix at runtime.
///
/// Only injects into HTML documents (detected by `<head>` presence). JS files are unchanged.
fn inject_base_path_global(html: &str, base_path: &str) -> String {
    let tag = "<head>";
    if let Some(pos) = html.find(tag) {
        let insert_at = pos + tag.len();
        let encoded = serde_json::to_string(base_path).unwrap_or_default();
        // Prevent </script> or <script> inside the JSON value from breaking the HTML
        let safe = encoded.replace('<', r"\u003c");
        let script = format!("<script>window.__REX_BASE_PATH={safe}</script>");
        format!("{}{}{}", &html[..insert_at], script, &html[insert_at..])
    } else {
        html.to_string()
    }
}

/// Recursively copy a directory's contents into a destination directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn route_to_file_path_root() {
        let output = Path::new("/out");
        assert_eq!(
            route_to_file_path(output, "/"),
            PathBuf::from("/out/index.html")
        );
    }

    #[test]
    fn route_to_file_path_simple() {
        let output = Path::new("/out");
        assert_eq!(
            route_to_file_path(output, "/about"),
            PathBuf::from("/out/about.html")
        );
    }

    #[test]
    fn route_to_file_path_nested() {
        let output = Path::new("/out");
        assert_eq!(
            route_to_file_path(output, "/docs/intro"),
            PathBuf::from("/out/docs/intro.html")
        );
    }

    #[test]
    fn validate_exportability_all_static() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/", "index.js", DataStrategy::None, false);
        manifest.add_page("/about", "about.js", DataStrategy::GetStaticProps, false);

        let warnings = validate_exportability(&manifest, false).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn validate_exportability_gssp_fails() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/", "index.js", DataStrategy::None, false);
        manifest.add_page("/dash", "dash.js", DataStrategy::GetServerSideProps, false);

        let err = validate_exportability(&manifest, false).unwrap_err();
        assert!(err.to_string().contains("getServerSideProps"));
    }

    #[test]
    fn validate_exportability_gssp_force() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/", "index.js", DataStrategy::None, false);
        manifest.add_page("/dash", "dash.js", DataStrategy::GetServerSideProps, false);

        let warnings = validate_exportability(&manifest, true).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("getServerSideProps"));
    }

    #[test]
    fn validate_exportability_dynamic_fails() {
        let mut manifest = AssetManifest::new("test".into());
        manifest.add_page("/blog/:slug", "slug.js", DataStrategy::None, true);

        let err = validate_exportability(&manifest, false).unwrap_err();
        assert!(err.to_string().contains("dynamic segments"));
    }

    #[test]
    fn write_html_creates_parent_dirs() {
        let tmp = std::env::temp_dir().join("rex_export_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let path = tmp.join("nested").join("page.html");
        write_html_file(&path, "<html></html>").unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<html></html>");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rewrite_asset_paths_empty_base() {
        let html = r#"<script src="/_rex/static/app.js"></script>"#;
        assert_eq!(rewrite_asset_paths(html, ""), html);
    }

    #[test]
    fn rewrite_asset_paths_with_base() {
        let html =
            r#"<link href="/_rex/static/style.css" /><script src="/_rex/router.js"></script>"#;
        let result = rewrite_asset_paths(html, "/rex");
        assert!(result.contains(r#"href="/rex/_rex/static/style.css""#));
        assert!(result.contains(r#"src="/rex/_rex/router.js""#));
    }

    #[test]
    fn rewrite_asset_paths_multiple_occurrences() {
        let html = "/_rex/static/a.js /_rex/static/b.js /_rex/data/c.json";
        let result = rewrite_asset_paths(html, "/docs");
        assert!(result.contains("/docs/_rex/static/a.js"));
        assert!(result.contains("/docs/_rex/static/b.js"));
        assert!(result.contains("/docs/_rex/data/c.json"));
    }

    #[test]
    fn rewrite_asset_paths_rewrites_nav_links() {
        let html = r#"<a href="/about">About</a><a href="/getting-started">Start</a>"#;
        let result = rewrite_asset_paths(html, "/rex");
        assert!(result.contains(r#"href="/rex/about""#));
        assert!(result.contains(r#"href="/rex/getting-started""#));
    }

    #[test]
    fn rewrite_asset_paths_preserves_external_links() {
        let html = r#"<a href="https://github.com">GH</a>"#;
        let result = rewrite_asset_paths(html, "/rex");
        // No <head> tag, so no script injection — external link preserved as-is
        assert!(result.contains(r#"href="https://github.com""#));
    }

    #[test]
    fn rewrite_asset_paths_no_double_prefix() {
        let html = r#"<link href="/_rex/static/s.css" /><a href="/about">A</a>"#;
        let result = rewrite_asset_paths(html, "/rex");
        assert!(result.contains(r#"href="/rex/_rex/static/s.css""#));
        assert!(result.contains(r#"href="/rex/about""#));
        assert!(!result.contains("/rex/rex/"));
    }

    #[test]
    fn rewrite_asset_paths_injects_base_path_global() {
        let html = r#"<html><head><meta charset="utf-8" /></head><body></body></html>"#;
        let result = rewrite_asset_paths(html, "/rex");
        assert!(result.contains(r#"<script>window.__REX_BASE_PATH="/rex"</script>"#));
        // Script is injected right after <head>
        let head_pos = result.find("<head>").unwrap();
        let script_pos = result.find("__REX_BASE_PATH").unwrap();
        assert!(script_pos > head_pos);
    }

    #[test]
    fn rewrite_asset_paths_no_injection_without_head() {
        // JS files (e.g. router.js) don't have <head> — no injection
        let js = r#"var x = "/_rex/data/test.json";"#;
        let result = rewrite_asset_paths(js, "/rex");
        assert!(!result.contains("__REX_BASE_PATH"));
    }

    #[test]
    fn inject_base_path_global_into_rsc_html() {
        let html =
            "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\" /></head><body></body></html>";
        let result = inject_base_path_global(html, "/docs");
        assert!(result.contains(r#"<head><script>window.__REX_BASE_PATH="/docs"</script><meta"#));
    }

    #[test]
    fn inject_base_path_global_escapes_special_chars() {
        let html = "<html><head></head><body></body></html>";
        let malicious = r#"/rex";</script><script>alert(1)//"#;
        let result = inject_base_path_global(html, malicious);
        assert!(result.contains("__REX_BASE_PATH="));
        // The </script> inside the value must be escaped so the HTML parser
        // doesn't close the script tag prematurely and execute injected code.
        // Count that there's exactly one <script> open and one </script> close.
        assert_eq!(result.matches("<script>").count(), 1);
        assert_eq!(result.matches("</script>").count(), 1);
    }
}
