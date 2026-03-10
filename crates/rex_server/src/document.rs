use std::collections::HashMap;

/// Descriptor from _document rendering (custom html/body attributes, extra head content)
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct DocumentDescriptor {
    #[serde(default, rename = "htmlAttrs")]
    pub html_attrs: HashMap<String, String>,
    #[serde(default, rename = "bodyAttrs")]
    pub body_attrs: HashMap<String, String>,
    #[serde(default, rename = "headContent")]
    pub head_content: String,
}

/// Parameters for assembling the final HTML document.
pub struct DocumentParams<'a> {
    pub ssr_html: &'a str,
    pub head_html: &'a str,
    pub props_json: &'a str,
    pub client_scripts: &'a [String],
    pub css_files: &'a [String],
    pub css_contents: &'a HashMap<String, String>,
    pub app_script: Option<&'a str>,
    pub is_dev: bool,
    pub doc_descriptor: Option<&'a DocumentDescriptor>,
    pub manifest_json: Option<&'a str>,
    pub font_preloads: &'a [String],
}

/// Assemble the final HTML document
pub fn assemble_document(params: &DocumentParams<'_>) -> String {
    let escaped_props = escape_script_content(params.props_json);

    let mut html = String::with_capacity(params.ssr_html.len() + 2048);

    html.push_str("<!DOCTYPE html>\n");

    // <html> tag with optional attributes from _document
    html.push_str("<html");
    if let Some(desc) = params.doc_descriptor {
        for (k, v) in &desc.html_attrs {
            html.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
    }
    html.push_str(">\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\" />\n");
    html.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");

    // Inject head elements from rex/head (title, meta, etc.)
    if !params.head_html.is_empty() {
        html.push_str("  ");
        html.push_str(params.head_html);
        html.push('\n');
    }

    // Inject extra head content from _document
    if let Some(desc) = params.doc_descriptor {
        if !desc.head_content.is_empty() {
            html.push_str("  ");
            html.push_str(&desc.head_content);
            html.push('\n');
        }
    }

    // Font preloads: start fetching font files early to prevent layout shift
    for font_file in params.font_preloads {
        html.push_str(&format!(
            "  <link rel=\"preload\" as=\"font\" type=\"font/woff2\" href=\"/_rex/static/{font_file}\" crossorigin />\n"
        ));
    }

    // CSS: inline content to avoid render-blocking network requests
    for css in params.css_files {
        if let Some(content) = params.css_contents.get(css) {
            html.push_str("  <style>");
            html.push_str(&escape_style_content(content));
            html.push_str("</style>\n");
        } else {
            // Fallback to link tag if content not available
            html.push_str(&format!(
                "  <link rel=\"stylesheet\" href=\"/_rex/static/{css}\" />\n"
            ));
        }
    }

    // <body> tag with optional attributes from _document
    html.push_str("</head>\n<body");
    if let Some(desc) = params.doc_descriptor {
        for (k, v) in &desc.body_attrs {
            html.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
    }
    html.push_str(">\n");

    // Main content
    html.push_str(&format!("  <div id=\"__rex\">{}</div>\n", params.ssr_html));

    // Props data for hydration
    html.push_str(&format!(
        "  <script id=\"__REX_DATA__\" type=\"application/json\">{escaped_props}</script>\n"
    ));

    // Route manifest for client-side navigation
    if let Some(manifest) = params.manifest_json {
        let escaped_manifest = escape_script_content(manifest);
        html.push_str(&format!(
            "  <script>window.__REX_MANIFEST__={escaped_manifest}</script>\n"
        ));
    }

    // _app client chunk (must load before page scripts for hydration wrapping)
    if let Some(app) = params.app_script {
        html.push_str(&format!(
            "  <script type=\"module\" src=\"/_rex/static/{app}\"></script>\n"
        ));
    }

    // Client chunks (ESM bundles produced by rolldown)
    for script in params.client_scripts {
        html.push_str(&format!(
            "  <script type=\"module\" src=\"/_rex/static/{script}\"></script>\n"
        ));
    }

    // Client-side router (must load after page scripts register __REX_RENDER__)
    if params.manifest_json.is_some() {
        html.push_str("  <script defer src=\"/_rex/router.js\"></script>\n");
    }

    // HMR client in dev mode
    if params.is_dev {
        html.push_str("  <script defer src=\"/_rex/hmr-client.js\"></script>\n");
    }

    html.push_str("</body>\n</html>");

    html
}

/// Escape content for safe embedding inside a `<script>` tag.
///
/// Replaces all `<` with `\u003c` so the HTML parser can never see a closing
/// `</script>` (or `</SCRIPT>`, `<!--`, etc.) inside the script block.
/// This is the same approach used by Next.js (`htmlEscapeJsonString`) and
/// the `serialize-javascript` npm package.
fn escape_script_content(s: &str) -> String {
    s.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Escape content for safe embedding inside a `<style>` tag.
///
/// Replaces all `<` so the HTML parser can never see a closing `</style>`
/// tag (case-insensitive) inside the style block.
fn escape_style_content(s: &str) -> String {
    s.replace('<', "\\u003c")
}

/// Escape a string for use as an HTML attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Assemble the HTML head shell — everything from doctype through opening `<body>` tag.
///
/// This is flushed to the browser immediately so it can start fetching CSS/JS
/// resources while the server renders the page body in V8.
pub fn assemble_head_shell(
    css_files: &[String],
    css_contents: &HashMap<String, String>,
    shared_chunks: &[String],
    app_script: Option<&str>,
    client_scripts: &[String],
    doc_descriptor: Option<&DocumentDescriptor>,
    font_preloads: &[String],
) -> String {
    let mut html = String::with_capacity(2048);

    html.push_str("<!DOCTYPE html>\n");

    // <html> tag with optional attributes from _document
    html.push_str("<html");
    if let Some(desc) = doc_descriptor {
        for (k, v) in &desc.html_attrs {
            html.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
    }
    html.push_str(">\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\" />\n");
    html.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");

    // Inject extra head content from _document
    if let Some(desc) = doc_descriptor {
        if !desc.head_content.is_empty() {
            html.push_str("  ");
            html.push_str(&desc.head_content);
            html.push('\n');
        }
    }

    // Font preloads: start fetching font files early to prevent layout shift
    for font_file in font_preloads {
        html.push_str(&format!(
            "  <link rel=\"preload\" as=\"font\" type=\"font/woff2\" href=\"/_rex/static/{font_file}\" crossorigin />\n"
        ));
    }

    // CSS: inline content to avoid render-blocking network requests
    for css in css_files {
        if let Some(content) = css_contents.get(css) {
            html.push_str("  <style>");
            html.push_str(&escape_style_content(content));
            html.push_str("</style>\n");
        } else {
            html.push_str(&format!(
                "  <link rel=\"stylesheet\" href=\"/_rex/static/{css}\" />\n"
            ));
        }
    }

    // Modulepreload hints: browser starts fetching + compiling JS immediately,
    // eliminating the import waterfall where entry modules must be fetched and
    // parsed before shared dependencies (React, etc.) are discovered.
    for chunk in shared_chunks {
        html.push_str(&format!(
            "  <link rel=\"modulepreload\" href=\"/_rex/static/{chunk}\" />\n"
        ));
    }
    if let Some(app) = app_script {
        html.push_str(&format!(
            "  <link rel=\"modulepreload\" href=\"/_rex/static/{app}\" />\n"
        ));
    }
    for script in client_scripts {
        html.push_str(&format!(
            "  <link rel=\"modulepreload\" href=\"/_rex/static/{script}\" />\n"
        ));
    }

    html.push_str("</head>\n<body");
    if let Some(desc) = doc_descriptor {
        for (k, v) in &desc.body_attrs {
            html.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
    }
    html.push_str(">\n");

    html
}

/// Assemble the body content and closing tags, sent after SSR render completes.
pub fn assemble_body_tail(
    ssr_html: &str,
    head_html: &str,
    props_json: &str,
    client_scripts: &[String],
    app_script: Option<&str>,
    is_dev: bool,
    manifest_json: Option<&str>,
) -> String {
    let escaped_props = escape_script_content(props_json);

    let mut html = String::with_capacity(ssr_html.len() + 1024);

    // Dynamic head elements from rex/head (title, meta, etc.)
    // Placed at top of body — browsers handle these correctly
    if !head_html.is_empty() {
        html.push_str("  ");
        html.push_str(head_html);
        html.push('\n');
    }

    // Main content
    html.push_str(&format!("  <div id=\"__rex\">{ssr_html}</div>\n"));

    // Props data for hydration
    html.push_str(&format!(
        "  <script id=\"__REX_DATA__\" type=\"application/json\">{escaped_props}</script>\n"
    ));

    // Route manifest for client-side navigation
    if let Some(manifest) = manifest_json {
        let escaped_manifest = escape_script_content(manifest);
        html.push_str(&format!(
            "  <script>window.__REX_MANIFEST__={escaped_manifest}</script>\n"
        ));
    }

    // _app client chunk (must load before page scripts for hydration wrapping)
    if let Some(app) = app_script {
        html.push_str(&format!(
            "  <script type=\"module\" src=\"/_rex/static/{app}\"></script>\n"
        ));
    }

    // Client chunks (ESM bundles produced by rolldown)
    for script in client_scripts {
        html.push_str(&format!(
            "  <script type=\"module\" src=\"/_rex/static/{script}\"></script>\n"
        ));
    }

    // Client-side router (must load after page scripts register __REX_RENDER__)
    if manifest_json.is_some() {
        html.push_str("  <script defer src=\"/_rex/router.js\"></script>\n");
    }

    // HMR client in dev mode
    if is_dev {
        html.push_str("  <script defer src=\"/_rex/hmr-client.js\"></script>\n");
    }

    html.push_str("</body>\n</html>");

    html
}

/// Parameters for assembling an RSC HTML document.
pub struct RscDocumentParams<'a> {
    /// Server-rendered HTML body from the RSC two-pass render
    pub ssr_html: &'a str,
    /// Head elements from server rendering
    pub head_html: &'a str,
    /// Flight data for client hydration
    pub flight_data: &'a str,
    /// Client component chunks to load
    pub client_chunks: &'a [String],
    /// Client reference manifest JSON (maps ref IDs to chunk URLs)
    pub client_manifest_json: &'a str,
    /// CSS filenames to include (global + per-route)
    pub css_files: &'a [String],
    /// CSS file contents for inlining (filename -> content)
    pub css_contents: &'a HashMap<String, String>,
    pub is_dev: bool,
    pub manifest_json: Option<&'a str>,
}

/// Assemble an RSC HTML document (non-streaming path).
///
/// Delegates to the streaming functions for consistency.
pub fn assemble_rsc_document(params: &RscDocumentParams<'_>) -> String {
    let mut html = String::new();
    html.push_str(&assemble_rsc_head_shell(
        params.client_chunks,
        params.client_manifest_json,
        params.css_files,
        params.css_contents,
    ));
    html.push_str(&assemble_rsc_body_tail(
        params.ssr_html,
        params.head_html,
        params.flight_data,
        params.client_chunks,
        params.client_manifest_json,
        params.is_dev,
        params.manifest_json,
    ));
    html
}

/// Assemble the RSC head shell — flushed to the browser immediately while V8 renders.
///
/// Emits the full `<head>` (meta, CSS, modulepreloads) and opens `<body>` with the
/// module map and webpack shims. This lets the browser start fetching client
/// chunks while V8 is still rendering the body HTML.
pub fn assemble_rsc_head_shell(
    client_chunks: &[String],
    client_manifest_json: &str,
    css_files: &[String],
    css_contents: &HashMap<String, String>,
) -> String {
    let mut html = String::with_capacity(2048);
    html.push_str("<!DOCTYPE html>\n<html><head>");
    html.push_str("<meta charset=\"utf-8\" />");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />");

    // CSS: inline content to avoid render-blocking network requests
    for css in css_files {
        if let Some(content) = css_contents.get(css) {
            html.push_str("<style>");
            html.push_str(&escape_style_content(content));
            html.push_str("</style>");
        } else {
            html.push_str(&format!(
                "<link rel=\"stylesheet\" href=\"/_rex/static/{css}\" />"
            ));
        }
    }

    for chunk in client_chunks {
        html.push_str(&format!(
            "<link rel=\"modulepreload\" href=\"/_rex/static/{chunk}\" />"
        ));
    }

    html.push_str("</head><body>");

    // Module map — must be available before client chunks load
    let escaped_manifest = escape_script_content(client_manifest_json);
    html.push_str(&format!(
        "<script>window.__REX_RSC_MODULE_MAP__={escaped_manifest}</script>"
    ));

    // Webpack shims — react-server-dom-webpack/client accesses __webpack_require__ during init
    html.push_str(
        "<script>\
         var __rexModuleCache={};\
         globalThis.__webpack_require__=function(id){return __rexModuleCache[id]||{}};\
         globalThis.__webpack_require__.u=function(c){return c};\
         globalThis.__webpack_chunk_load__=function(c){\
           if(__rexModuleCache[c])return Promise.resolve();\
           return import(c).then(function(m){__rexModuleCache[c]=m})\
         };\
         window.__rexModuleCache=__rexModuleCache;\
         </script>",
    );

    html
}

/// Assemble the RSC body tail — sent after V8 render completes.
///
/// The head shell (from `assemble_rsc_head_shell`) already emitted
/// `<!DOCTYPE html><html><head>...</head><body>` plus the module map and
/// webpack shims. This function outputs:
///   SSR body content + scripts + `</body></html>`
///
/// The SSR HTML from V8 may contain `<html>...<body>...</body></html>` from
/// the root layout; we strip those outer tags and extract only the body content.
pub fn assemble_rsc_body_tail(
    ssr_html: &str,
    head_html: &str,
    flight_data: &str,
    client_chunks: &[String],
    _client_manifest_json: &str,
    is_dev: bool,
    manifest_json: Option<&str>,
) -> String {
    // Extract the inner body content from the SSR HTML.
    // The root layout typically renders <html><head></head><body>...</body></html>.
    let body_content = extract_body_content(ssr_html);

    let mut html = String::with_capacity(body_content.len() + 2048);

    // Metadata head elements (title, meta, link, etc.) from generateMetadata / metadata exports.
    // Emitted at the top of the body — browsers relocate these to <head> automatically.
    // This is the standard approach for streaming SSR (used by Next.js and others).
    if !head_html.is_empty() {
        html.push_str(head_html);
    }

    // RSC hydrates the full document (hydrateRoot(document, tree)),
    // so emit the body content directly without a wrapper div.
    html.push_str(body_content);

    // Inline flight data for hydration
    let escaped_flight = escape_script_content(flight_data);
    html.push_str(&format!(
        "<script id=\"__REX_RSC_DATA__\" type=\"text/rsc\">{escaped_flight}</script>"
    ));

    // Route manifest
    if let Some(manifest) = manifest_json {
        let escaped = escape_script_content(manifest);
        html.push_str(&format!(
            "<script>window.__REX_MANIFEST__={escaped}</script>"
        ));
    }

    // Client component chunks (includes the RSC hydration entry)
    for chunk in client_chunks {
        html.push_str(&format!(
            "<script type=\"module\" src=\"/_rex/static/{chunk}\"></script>"
        ));
    }

    // Client-side router
    if manifest_json.is_some() {
        html.push_str("<script defer src=\"/_rex/router.js\"></script>");
    }

    // HMR in dev
    if is_dev {
        html.push_str("<script defer src=\"/_rex/hmr-client.js\"></script>");
    }

    html.push_str("</body></html>");
    html
}

/// Extract the content between `<body>` and `</body>` from SSR HTML.
/// Falls back to the entire string if no body tags are found.
fn extract_body_content(ssr_html: &str) -> &str {
    if let Some(body_start) = ssr_html.find("<body") {
        if let Some(tag_end) = ssr_html[body_start..].find('>') {
            let content_start = body_start + tag_end + 1;
            if let Some(body_end) = ssr_html.rfind("</body>") {
                return &ssr_html[content_start..body_end];
            }
        }
    }
    ssr_html
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
