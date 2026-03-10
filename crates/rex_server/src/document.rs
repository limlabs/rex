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
            html.push_str(content);
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

/// Escape content for safe embedding inside a <script> tag.
/// Prevents XSS via </script> injection.
pub(crate) fn escape_script_content(s: &str) -> String {
    s.replace("</script", "<\\/script")
        .replace("<!--", "<\\!--")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Escape content for safe embedding inside a <style> tag.
/// Prevents injection via </style> sequences in CSS content.
pub(crate) fn escape_style_content(s: &str) -> String {
    s.replace("</style", "<\\/style")
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn escape_script_content_script_tag() {
        let input = r#"var x = "</script>";"#;
        let result = escape_script_content(input);
        assert!(result.contains(r"<\/script"));
        assert!(!result.contains("</script"));
    }

    #[test]
    fn escape_script_content_html_comment() {
        let input = "<!-- comment -->";
        let result = escape_script_content(input);
        assert!(result.contains(r"<\!--"));
        assert!(!result.contains("<!--"));
    }

    #[test]
    fn escape_script_content_line_separators() {
        let input = "a\u{2028}b\u{2029}c";
        let result = escape_script_content(input);
        assert!(result.contains("\\u2028"));
        assert!(result.contains("\\u2029"));
        assert!(!result.contains('\u{2028}'));
        assert!(!result.contains('\u{2029}'));
    }

    #[test]
    fn escape_script_content_passthrough() {
        let input = r#"{"key": "value", "num": 42}"#;
        let result = escape_script_content(input);
        assert_eq!(result, input);
    }
}
