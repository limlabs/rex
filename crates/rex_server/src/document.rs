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
fn escape_script_content(s: &str) -> String {
    s.replace("</script", "<\\/script")
        .replace("<!--", "<\\!--")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
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
    pub is_dev: bool,
    pub manifest_json: Option<&'a str>,
}

/// Assemble an RSC HTML document (non-streaming path).
///
/// Delegates to the streaming functions for consistency.
pub fn assemble_rsc_document(params: &RscDocumentParams<'_>) -> String {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html>\n");
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
/// For RSC routes, the root layout renders `<html>` and `<body>`, so the SSR
/// output IS the document. We just emit the doctype here; the body tail injects
/// scripts/meta into the SSR HTML's `</head>` and `</body>`.
pub fn assemble_rsc_head_shell(_client_chunks: &[String], _client_manifest_json: &str) -> String {
    "<!DOCTYPE html>\n".to_string()
}

/// Assemble the RSC body tail — sent after V8 render completes.
///
/// For RSC routes, the SSR HTML from `renderToString` already contains
/// `<html><head></head><body>...</body></html>` from the root layout.
/// We inject Rex's meta/preloads into `</head>` and scripts into `</body>`
/// rather than wrapping the HTML in another shell (which would create
/// invalid nested `<html>` elements and break hydration).
pub fn assemble_rsc_body_tail(
    ssr_html: &str,
    head_html: &str,
    flight_data: &str,
    client_chunks: &[String],
    client_manifest_json: &str,
    is_dev: bool,
    manifest_json: Option<&str>,
) -> String {
    // Build head injections (meta, preloads, dynamic head elements)
    let mut head_inject = String::new();
    head_inject.push_str("<meta charset=\"utf-8\" />");
    head_inject
        .push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />");
    if !head_html.is_empty() {
        head_inject.push_str(head_html);
    }
    for chunk in client_chunks {
        head_inject.push_str(&format!(
            "<link rel=\"modulepreload\" href=\"/_rex/static/{chunk}\" />"
        ));
    }

    // Build body injections (scripts)
    let mut body_inject = String::new();

    // Client reference manifest
    let escaped_manifest = escape_script_content(client_manifest_json);
    body_inject.push_str(&format!(
        "<script>window.__REX_RSC_MODULE_MAP__={escaped_manifest}</script>"
    ));

    // Webpack shims — must be set before <script type="module"> loads because
    // react-server-dom-webpack/client accesses __webpack_require__ during init
    body_inject.push_str(
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

    // Inline flight data for hydration
    let escaped_flight = escape_script_content(flight_data);
    body_inject.push_str(&format!(
        "<script id=\"__REX_RSC_DATA__\" type=\"text/rsc\">{escaped_flight}</script>"
    ));

    // Route manifest
    if let Some(manifest) = manifest_json {
        let escaped = escape_script_content(manifest);
        body_inject.push_str(&format!(
            "<script>window.__REX_MANIFEST__={escaped}</script>"
        ));
    }

    // Client component chunks (includes the RSC hydration entry)
    for chunk in client_chunks {
        body_inject.push_str(&format!(
            "<script type=\"module\" src=\"/_rex/static/{chunk}\"></script>"
        ));
    }

    // Client-side router
    if manifest_json.is_some() {
        body_inject.push_str("<script defer src=\"/_rex/router.js\"></script>");
    }

    // HMR in dev
    if is_dev {
        body_inject.push_str("<script defer src=\"/_rex/hmr-client.js\"></script>");
    }

    // Inject into SSR HTML if it contains <html> (RSC root layout convention)
    if ssr_html.contains("</head>") && ssr_html.contains("</body>") {
        ssr_html
            .replacen("</head>", &format!("{head_inject}</head>"), 1)
            .replacen("</body>", &format!("{body_inject}</body>"), 1)
    } else {
        // Fallback for layouts without <html>/<body>: wrap in a shell
        let mut html = String::with_capacity(ssr_html.len() + 2048);
        html.push_str("<html><head>");
        html.push_str(&head_inject);
        html.push_str("</head><body>");
        html.push_str(ssr_html);
        html.push_str(&body_inject);
        html.push_str("</body></html>");
        html
    }
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

    #[test]
    fn rsc_head_shell_emits_doctype() {
        let chunks = vec!["rsc/chunk-react-abc123.js".to_string()];
        let manifest = r#"{"refs":{}}"#;
        let html = assemble_rsc_head_shell(&chunks, manifest);

        assert!(html.contains("<!DOCTYPE html>"));
        // Head shell is minimal — preloads are injected into SSR HTML by body tail
        assert!(!html.contains("<head>"));
    }

    #[test]
    fn rsc_body_tail_injects_into_ssr_html() {
        let ssr = "<html><head></head><body><div>Hello</div></body></html>";
        let html = assemble_rsc_body_tail(
            ssr,
            "",
            "0:\"hello\"\n",
            &["rsc/component-abc.js".to_string()],
            "{}",
            false,
            None,
        );

        // SSR HTML structure preserved (no nested <html>, no wrapper div)
        assert!(!html.contains("<div id=\"__rex\">"));
        // Meta tags injected into <head>
        assert!(html.contains("<meta charset=\"utf-8\" />"));
        // Head injection goes before </head>
        let meta_pos = html.find("<meta charset").unwrap();
        let head_close_pos = html.find("</head>").unwrap();
        assert!(meta_pos < head_close_pos);
        // Flight data injected before </body>
        assert!(html.contains("__REX_RSC_DATA__"));
        assert!(html.contains("0:\"hello\""));
        // Script tags injected before </body>
        assert!(html.contains(
            r#"<script type="module" src="/_rex/static/rsc/component-abc.js"></script>"#
        ));
        assert!(html.contains("</body></html>"));
    }

    #[test]
    fn rsc_body_tail_fallback_without_html_wrapper() {
        // SSR HTML without <html>/<body> (e.g., layout without root element)
        let ssr = "<div>Hello</div>";
        let html = assemble_rsc_body_tail(ssr, "", "0:\"hi\"", &[], "{}", false, None);

        // Falls back to wrapping in a shell
        assert!(html.contains("<html><head>"));
        assert!(html.contains("<div>Hello</div>"));
        assert!(html.contains("</body></html>"));
    }

    #[test]
    fn rsc_body_tail_script_ordering() {
        let ssr = "<html><head></head><body><p>test</p></body></html>";
        let html = assemble_rsc_body_tail(
            ssr,
            "",
            "0:{}",
            &["rsc/comp.js".to_string()],
            "{}",
            true,
            Some(r#"{"routes":{}}"#),
        );

        // All body scripts injected before </body> in correct order:
        // module map → flight data → route manifest → component scripts → router → HMR
        let map_pos = html.find("__REX_RSC_MODULE_MAP__").unwrap();
        let flight_pos = html.find("__REX_RSC_DATA__").unwrap();
        // Find the script tag occurrence of comp.js (not the modulepreload in head)
        let comp_script_pos = html
            .find(r#"<script type="module" src="/_rex/static/rsc/comp.js">"#)
            .unwrap();
        let router_pos = html.find("router.js").unwrap();
        let hmr_pos = html.find("hmr-client.js").unwrap();

        assert!(map_pos < flight_pos);
        assert!(flight_pos < comp_script_pos);
        assert!(comp_script_pos < router_pos);
        assert!(router_pos < hmr_pos);
    }

    #[test]
    fn rsc_body_tail_includes_module_map() {
        let ssr = "<html><head></head><body><p>Hi</p></body></html>";
        let manifest = r#"{"entries":{"abc":{"chunk_url":"/c.js","export_name":"default"}}}"#;
        let html = assemble_rsc_body_tail(ssr, "", "0:\"x\"", &[], manifest, false, None);

        assert!(html.contains("__REX_RSC_MODULE_MAP__"));
        assert!(html.contains("abc"));
    }
}
