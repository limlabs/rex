use std::collections::HashMap;

/// Descriptor from _document rendering (custom html/body attributes, extra head content)
#[derive(Debug, Default, serde::Deserialize)]
pub struct DocumentDescriptor {
    #[serde(default, rename = "htmlAttrs")]
    pub html_attrs: HashMap<String, String>,
    #[serde(default, rename = "bodyAttrs")]
    pub body_attrs: HashMap<String, String>,
    #[serde(default, rename = "headContent")]
    pub head_content: String,
}

/// Assemble the final HTML document
pub fn assemble_document(
    ssr_html: &str,
    head_html: &str,
    props_json: &str,
    client_scripts: &[String],
    css_files: &[String],
    css_contents: &HashMap<String, String>,
    app_script: Option<&str>,
    is_dev: bool,
    doc_descriptor: Option<&DocumentDescriptor>,
    manifest_json: Option<&str>,
) -> String {
    let escaped_props = escape_script_content(props_json);

    let mut html = String::with_capacity(ssr_html.len() + 2048);

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

    // Inject head elements from rex/head (title, meta, etc.)
    if !head_html.is_empty() {
        html.push_str("  ");
        html.push_str(head_html);
        html.push('\n');
    }

    // Inject extra head content from _document
    if let Some(desc) = doc_descriptor {
        if !desc.head_content.is_empty() {
            html.push_str("  ");
            html.push_str(&desc.head_content);
            html.push('\n');
        }
    }

    // CSS: inline content to avoid render-blocking network requests
    for css in css_files {
        if let Some(content) = css_contents.get(css) {
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
    if let Some(desc) = doc_descriptor {
        for (k, v) in &desc.body_attrs {
            html.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
    }
    html.push_str(">\n");

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

/// Escape content for safe embedding inside a <script> tag.
/// Prevents XSS via </script> injection.
fn escape_script_content(s: &str) -> String {
    s.replace("</script", "<\\/script")
        .replace("<!--", "<\\!--")
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

    // CSS: inline content to avoid render-blocking network requests
    for css in css_files {
        if let Some(content) = css_contents.get(css) {
            html.push_str("  <style>");
            html.push_str(content);
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
