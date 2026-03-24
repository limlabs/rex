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
    /// Import map JSON for unbundled dev serving (dev mode only).
    /// When Some, script srcs are treated as full URLs (not prefixed with /_rex/static/).
    pub import_map_json: Option<&'a str>,
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

    // Import map for unbundled dev serving (must appear before any module scripts)
    if let Some(import_map) = params.import_map_json {
        html.push_str(&format!(
            "  <script type=\"importmap\">{import_map}</script>\n"
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

    let unbundled = params.import_map_json.is_some();

    // _app client chunk (must load before page scripts for hydration wrapping)
    if let Some(app) = params.app_script {
        if unbundled {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"{app}\"></script>\n"
            ));
        } else {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"/_rex/static/{app}\"></script>\n"
            ));
        }
    }

    // Client chunks (ESM bundles produced by rolldown, or /_rex/entry/ URLs in dev)
    for script in params.client_scripts {
        if unbundled {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"{script}\"></script>\n"
            ));
        } else {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"/_rex/static/{script}\"></script>\n"
            ));
        }
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
pub(crate) fn escape_script_content(s: &str) -> String {
    s.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Escape content for safe embedding inside a `<style>` tag.
///
/// Replaces all `<` so the HTML parser can never see a closing `</style>`
/// tag (case-insensitive) inside the style block.
pub(crate) fn escape_style_content(s: &str) -> String {
    s.replace('<', "\\u003c")
}

/// Escape a string for safe embedding inside a JavaScript single-quoted string literal.
/// Handles backslashes, single quotes, newlines, and `</script>` sequences.
pub(crate) fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace("</script", "<\\/script")
}

/// Escape a string for use as an HTML attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Sanitize a raw tag attribute string for safe insertion into an HTML opening tag.
///
/// `extract_*_tag_attrs` already stops at the first `>`, so by construction the
/// string cannot contain `>`. This is a defense-in-depth measure that strips any
/// `>` character to prevent premature tag closure.
pub(crate) fn sanitize_tag_attrs(s: &str) -> String {
    s.replace('>', "")
}

/// Parameters for assembling the HTML head shell.
pub struct HeadShellParams<'a> {
    pub css_files: &'a [String],
    pub css_contents: &'a HashMap<String, String>,
    pub shared_chunks: &'a [String],
    pub app_script: Option<&'a str>,
    pub client_scripts: &'a [String],
    pub doc_descriptor: Option<&'a DocumentDescriptor>,
    pub font_preloads: &'a [String],
    pub import_map_json: Option<&'a str>,
}

/// Parameters for assembling the body tail.
pub struct BodyTailParams<'a> {
    pub ssr_html: &'a str,
    pub head_html: &'a str,
    pub props_json: &'a str,
    pub client_scripts: &'a [String],
    pub app_script: Option<&'a str>,
    pub is_dev: bool,
    pub manifest_json: Option<&'a str>,
    pub import_map_json: Option<&'a str>,
}

/// Assemble the HTML head shell — everything from doctype through opening `<body>` tag.
///
/// This is flushed to the browser immediately so it can start fetching CSS/JS
/// resources while the server renders the page body in V8.
pub fn assemble_head_shell(params: &HeadShellParams<'_>) -> String {
    let mut html = String::with_capacity(2048);

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

    // Inject extra head content from _document
    if let Some(desc) = params.doc_descriptor {
        if !desc.head_content.is_empty() {
            html.push_str("  ");
            html.push_str(&desc.head_content);
            html.push('\n');
        }
    }

    // Import map for unbundled dev serving (must appear before any module scripts)
    if let Some(import_map) = params.import_map_json {
        html.push_str(&format!(
            "  <script type=\"importmap\">{import_map}</script>\n"
        ));
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
            html.push_str(&format!(
                "  <link rel=\"stylesheet\" href=\"/_rex/static/{css}\" />\n"
            ));
        }
    }

    let unbundled = params.import_map_json.is_some();

    // Modulepreload hints: browser starts fetching + compiling JS immediately,
    // eliminating the import waterfall where entry modules must be fetched and
    // parsed before shared dependencies (React, etc.) are discovered.
    if unbundled {
        // In unbundled dev mode, preload the core dep modules
        for dep in &["react.js", "react__jsx-runtime.js", "react-dom__client.js"] {
            html.push_str(&format!(
                "  <link rel=\"modulepreload\" href=\"/_rex/dep/{dep}\" />\n"
            ));
        }
    } else {
        for chunk in params.shared_chunks {
            html.push_str(&format!(
                "  <link rel=\"modulepreload\" href=\"/_rex/static/{chunk}\" />\n"
            ));
        }
    }
    if let Some(app) = params.app_script {
        if unbundled {
            html.push_str(&format!(
                "  <link rel=\"modulepreload\" href=\"{app}\" />\n"
            ));
        } else {
            html.push_str(&format!(
                "  <link rel=\"modulepreload\" href=\"/_rex/static/{app}\" />\n"
            ));
        }
    }
    for script in params.client_scripts {
        if unbundled {
            html.push_str(&format!(
                "  <link rel=\"modulepreload\" href=\"{script}\" />\n"
            ));
        } else {
            html.push_str(&format!(
                "  <link rel=\"modulepreload\" href=\"/_rex/static/{script}\" />\n"
            ));
        }
    }

    html.push_str("</head>\n<body");
    if let Some(desc) = params.doc_descriptor {
        for (k, v) in &desc.body_attrs {
            html.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
    }
    html.push_str(">\n");

    html
}

/// Assemble the body content and closing tags, sent after SSR render completes.
pub fn assemble_body_tail(params: &BodyTailParams<'_>) -> String {
    let escaped_props = escape_script_content(params.props_json);

    let mut html = String::with_capacity(params.ssr_html.len() + 1024);

    // Dynamic head elements from rex/head (title, meta, etc.)
    // Placed at top of body — browsers handle these correctly
    if !params.head_html.is_empty() {
        html.push_str("  ");
        html.push_str(params.head_html);
        html.push('\n');
    }

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

    let unbundled = params.import_map_json.is_some();

    // _app client chunk (must load before page scripts for hydration wrapping)
    if let Some(app) = params.app_script {
        if unbundled {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"{app}\"></script>\n"
            ));
        } else {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"/_rex/static/{app}\"></script>\n"
            ));
        }
    }

    // Client chunks (ESM bundles produced by rolldown, or /_rex/entry/ URLs in dev)
    for script in params.client_scripts {
        if unbundled {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"{script}\"></script>\n"
            ));
        } else {
            html.push_str(&format!(
                "  <script type=\"module\" src=\"/_rex/static/{script}\"></script>\n"
            ));
        }
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

/// Extract the raw attribute string from the `<html ...>` tag in SSR HTML.
/// Returns an empty string if no attributes are found.
pub fn extract_html_tag_attrs(ssr_html: &str) -> &str {
    if let Some(start) = ssr_html.find("<html") {
        let after = &ssr_html[start + 5..]; // skip "<html"
        if let Some(end) = after.find('>') {
            let attrs = after[..end].trim();
            if !attrs.is_empty() {
                return attrs;
            }
        }
    }
    ""
}

/// Extract the raw attribute string from the `<body ...>` tag in SSR HTML.
/// Returns an empty string if no attributes are found.
pub fn extract_body_tag_attrs(ssr_html: &str) -> &str {
    if let Some(start) = ssr_html.find("<body") {
        let after = &ssr_html[start + 5..]; // skip "<body"
        if let Some(end) = after.find('>') {
            let attrs = after[..end].trim();
            if !attrs.is_empty() {
                return attrs;
            }
        }
    }
    ""
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
