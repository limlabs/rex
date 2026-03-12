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
    /// Append `.html` extensions to internal navigation links.
    ///
    /// Most static hosts (GitHub Pages, Netlify, Vercel, Cloudflare Pages) serve
    /// `about.html` for requests to `/about`, so this is **off by default** for
    /// clean URLs.  Enable with `--html-extensions` for hosts that require the
    /// explicit extension (e.g. S3, basic nginx).
    pub html_extensions: bool,
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

    // Create data directory for client-side navigation
    let data_dir = output
        .join("_rex")
        .join("data")
        .join(&ctx.manifest.build_id);
    std::fs::create_dir_all(&data_dir)?;

    for (pattern, page) in &prerendered_pages {
        let file_path = route_to_file_path(output, pattern);
        let html = if config.html_extensions {
            rewrite_nav_links_to_html(&page.html)
        } else {
            page.html.to_string()
        };
        let html = inject_static_export_flag(&html, config.html_extensions);
        let html = rewrite_asset_paths(&html, &config.base_path);
        write_html_file(&file_path, &html)?;

        // Write static data JSON for client-side navigation
        let data_json = format!(r#"{{"props":{}}}"#, page.props_json);
        let data_path = route_to_data_path(&data_dir, pattern);
        write_html_file(&data_path, &data_json)?;

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

    // Create RSC flight data directory for client-side navigation
    let rsc_dir = output.join("_rex").join("rsc").join(&ctx.manifest.build_id);
    if !prerendered_app.is_empty() {
        std::fs::create_dir_all(&rsc_dir)?;
    }

    for (pattern, rendered) in &prerendered_app {
        let file_path = route_to_file_path(output, pattern);
        let html = if config.html_extensions {
            rewrite_nav_links_to_html(&rendered.html)
        } else {
            rendered.html.clone()
        };
        let html = inject_static_export_flag(&html, config.html_extensions);
        let html = rewrite_asset_paths(&html, &config.base_path);
        write_html_file(&file_path, &html)?;

        // Write RSC flight data for client-side navigation
        let flight_data = if config.html_extensions {
            rewrite_nav_links_to_html(&rendered.flight)
        } else {
            rendered.flight.clone()
        };
        let rsc_path = route_to_rsc_path(&rsc_dir, pattern);
        write_html_file(&rsc_path, &flight_data)?;

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
    let combined = format!("{shell}{tail}");
    let html = if config.html_extensions {
        rewrite_nav_links_to_html(&combined)
    } else {
        combined
    };
    let html = inject_static_export_flag(&html, config.html_extensions);
    let html = rewrite_asset_paths(&html, &config.base_path);

    let path = config.output_dir.join("404.html");
    if let Err(e) = std::fs::write(&path, &html) {
        warn!(error = %e, "Failed to write 404.html");
    } else {
        debug!("Exported 404.html");
        result.pages_exported += 1;
    }
}

/// Convert a route pattern like "/about" to a file path like "output/about/index.html".
///
/// Uses the `directory/index.html` convention for all non-root routes.  This is
/// critical for static hosts (GitHub Pages, S3, Cloudflare Pages) which redirect
/// `/path` to `/path/` when a same-named directory exists — causing a 404 if
/// the file was stored as `path.html` instead of `path/index.html`.
fn route_to_file_path(output: &Path, pattern: &str) -> PathBuf {
    if pattern == "/" {
        output.join("index.html")
    } else {
        // "/about" -> "about/index.html", "/docs/intro" -> "docs/intro/index.html"
        let stripped = pattern.trim_start_matches('/');
        output.join(stripped).join("index.html")
    }
}

/// Convert a route pattern to a data JSON file path for client-side navigation.
///
/// Maps the same URL structure the client router uses to fetch page data:
///   `/` → `data_dir/.json`
///   `/about` → `data_dir/about.json`
///   `/docs/intro` → `data_dir/docs/intro.json`
fn route_to_data_path(data_dir: &Path, pattern: &str) -> PathBuf {
    // Client fetches: /_rex/data/{buildId}{pathname}.json
    // Root uses index.json to avoid dotfile (.json) which static servers skip
    if pattern == "/" {
        data_dir.join("index.json")
    } else {
        let stripped = pattern.trim_start_matches('/');
        data_dir.join(format!("{stripped}.json"))
    }
}

/// Convert a route pattern to an RSC flight data file path.
///
/// Uses `.rsc` extension so root doesn't conflict with subdirectories:
///   `/` → `rsc_dir/.rsc`
///   `/about` → `rsc_dir/about.rsc`
///   `/docs/intro` → `rsc_dir/docs/intro.rsc`
fn route_to_rsc_path(rsc_dir: &Path, pattern: &str) -> PathBuf {
    // Root uses index.rsc to avoid dotfile (.rsc) which static servers skip
    if pattern == "/" {
        rsc_dir.join("index.rsc")
    } else {
        let stripped = pattern.trim_start_matches('/');
        rsc_dir.join(format!("{stripped}.rsc"))
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

/// Rewrite internal navigation links to use `.html` extensions for static file hosting.
///
/// Handles both HTML attributes (`href="/about"`) and JSON in RSC flight data
/// (`"href":"/about"`), so links survive React hydration.
///
/// Preserves:
/// - Root link: `"/"` (served by `index.html`)
/// - Asset links: `"/_rex/..."` (JS, CSS)
/// - External links: `"https://..."`, `"http://..."`
/// - Anchor links: `"#section"`
/// - Links with existing extensions: `"/file.css"`
fn rewrite_nav_links_to_html(html: &str) -> String {
    // Two patterns to match:
    //   HTML attrs: href="/path"
    //   JSON props: "href":"/path"
    const PATTERNS: &[&str] = &["href=\"/", "\"href\":\"/"];

    let mut result = String::with_capacity(html.len() + 512);
    let mut remaining = html;

    loop {
        // Find the earliest match of any pattern
        let mut best: Option<(usize, &str)> = None;
        for pat in PATTERNS {
            if let Some(idx) = remaining.find(pat) {
                if best.is_none_or(|(best_idx, _)| idx < best_idx) {
                    best = Some((idx, pat));
                }
            }
        }

        let (idx, pat) = match best {
            Some(b) => b,
            None => break,
        };

        // Push everything before this match
        result.push_str(&remaining[..idx]);
        remaining = &remaining[idx..];

        // The pattern includes the leading `/`. Everything from after `/` to the
        // closing `"` is the rest of the path value.
        let slash_pos = pat.len() - 1; // index of `/` in `remaining`
        let after_slash = &remaining[pat.len()..];

        let close = match after_slash.find('"') {
            Some(i) => i,
            None => {
                result.push_str(&remaining[..pat.len()]);
                remaining = &remaining[pat.len()..];
                continue;
            }
        };

        // Full path = "/" + after_slash[..close]
        let path = &remaining[slash_pos..pat.len() + close];

        // Split off fragment first (comes after query string in a URL)
        let (path_and_query, fragment) = match path.find('#') {
            Some(i) => (&path[..i], &path[i..]),
            None => (path, ""),
        };
        // Split off query string from the path
        let (base, query) = match path_and_query.find('?') {
            Some(i) => (&path_and_query[..i], &path_and_query[i..]),
            None => (path_and_query, ""),
        };

        // Decide if this path needs .html
        // Check only the final path segment for extensions
        let needs_html = path != "/"
            && !path.starts_with("/_rex/")
            && !path.starts_with("//")
            && !base.ends_with('/')
            && !base
                .rsplit_once('/')
                .map_or(base, |(_, last)| last)
                .contains('.');

        if needs_html {
            // Write everything up to the path value (prefix + opening quote)
            result.push_str(&remaining[..slash_pos]);
            result.push_str(base);
            result.push_str(".html");
            result.push_str(query);
            result.push_str(fragment);
            result.push('"');
        } else {
            // Keep as-is through closing quote
            result.push_str(&remaining[..pat.len() + close + 1]);
        }

        remaining = &remaining[pat.len() + close + 1..];
    }

    result.push_str(remaining);
    result
}

/// Inject `window.__REX_STATIC_EXPORT=true` so the client-side Link component
/// falls back to full-page navigation instead of RSC flight data fetching.
///
/// When `html_extensions` is true, also sets `window.__REX_STATIC_HTML_EXT=true`
/// so the Link component appends `.html` to internal hrefs at runtime.
fn inject_static_export_flag(html: &str, html_extensions: bool) -> String {
    let tag = "<head>";
    if let Some(pos) = html.find(tag) {
        let insert_at = pos + tag.len();
        let ext_part = if html_extensions {
            ";window.__REX_STATIC_HTML_EXT=true"
        } else {
            ""
        };
        let script = format!("<script>window.__REX_STATIC_EXPORT=true{ext_part}</script>");
        format!("{}{}{}", &html[..insert_at], script, &html[insert_at..])
    } else {
        html.to_string()
    }
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
#[path = "export_tests.rs"]
mod export_tests;
