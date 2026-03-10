use std::collections::HashMap;

use crate::document::{escape_script_content, escape_style_content};

/// Sanitize a string containing multiple HTML attributes (e.g. `lang="en" class="dark"`).
///
/// Parses individual attributes and escapes dangerous characters in attribute values
/// to prevent attribute injection attacks. Invalid or malformed attributes are
/// silently dropped to ensure the output is always safe.
///
/// # Examples
/// - `lang="en"` → `lang="en"`
/// - `lang="<script>"` → `lang="&lt;script&gt;"`
/// - `class='"><img src=x onerror=alert(1)'` → `class="&quot;&gt;&lt;img src=x onerror=alert(1)"`
fn sanitize_attrs(attrs: &str) -> String {
    if attrs.is_empty() {
        return String::new();
    }

    let mut result = Vec::new();
    let mut chars = attrs.chars().peekable();

    while let Some(&c) = chars.peek() {
        // Skip whitespace between attributes
        if c.is_whitespace() {
            chars.next();
            continue;
        }

        // Parse attribute name: ASCII letters, digits, hyphens, underscores, colons
        let name: String = chars
            .by_ref()
            .take_while(|&ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ':')
            .collect();

        if name.is_empty() {
            // Invalid character at start of attribute name — skip it to avoid infinite loop
            chars.next();
            continue;
        }

        // Skip whitespace around '='
        while chars.peek().is_some_and(|&ch| ch.is_whitespace()) {
            chars.next();
        }

        // Check for '='
        if chars.peek() != Some(&'=') {
            // Boolean attribute (no value) — include as-is
            result.push(name);
            continue;
        }
        chars.next(); // consume '='

        // Skip whitespace after '='
        while chars.peek().is_some_and(|&ch| ch.is_whitespace()) {
            chars.next();
        }

        // Parse attribute value
        let value = match chars.peek() {
            Some(&'"') => {
                chars.next(); // consume opening quote
                let v: String = chars.by_ref().take_while(|&ch| ch != '"').collect();
                v
            }
            Some(&'\'') => {
                chars.next(); // consume opening quote
                let v: String = chars.by_ref().take_while(|&ch| ch != '\'').collect();
                v
            }
            _ => {
                // Unquoted value — take until whitespace or end
                let v: String = chars
                    .by_ref()
                    .take_while(|&ch| !ch.is_whitespace())
                    .collect();
                v
            }
        };

        // Escape the value and emit with double quotes
        let escaped = escape_attr_value(&value);
        result.push(format!("{name}=\"{escaped}\""));
    }

    result.join(" ")
}

/// Escape a string for use as an HTML attribute value.
fn escape_attr_value(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
    /// Raw attribute string for `<html>` tag (e.g. `lang="en"`)
    pub html_attrs: &'a str,
    /// Raw attribute string for `<body>` tag (e.g. `class="bg-white"`)
    pub body_attrs: &'a str,
}

/// Assemble an RSC HTML document (non-streaming path).
///
/// Delegates to the streaming functions for consistency.
pub fn assemble_rsc_document(params: &RscDocumentParams<'_>) -> String {
    let mut html = String::new();
    html.push_str(&assemble_rsc_head_shell_with_attrs(
        params.client_chunks,
        params.client_manifest_json,
        params.css_files,
        params.css_contents,
        params.html_attrs,
        params.body_attrs,
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
    assemble_rsc_head_shell_with_attrs(
        client_chunks,
        client_manifest_json,
        css_files,
        css_contents,
        "",
        "",
    )
}

/// Like `assemble_rsc_head_shell` but allows passing through `<html>` and `<body>` attributes
/// extracted from the SSR output so the served HTML matches the RSC flight data.
fn assemble_rsc_head_shell_with_attrs(
    client_chunks: &[String],
    client_manifest_json: &str,
    css_files: &[String],
    css_contents: &HashMap<String, String>,
    html_attrs: &str,
    body_attrs: &str,
) -> String {
    let mut html = String::with_capacity(2048);
    let sanitized_html_attrs = sanitize_attrs(html_attrs);
    if sanitized_html_attrs.is_empty() {
        html.push_str("<!DOCTYPE html>\n<html><head>");
    } else {
        html.push_str("<!DOCTYPE html>\n<html ");
        html.push_str(&sanitized_html_attrs);
        html.push_str("><head>");
    }
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

    let sanitized_body_attrs = sanitize_attrs(body_attrs);
    if sanitized_body_attrs.is_empty() {
        html.push_str("</head><body>");
    } else {
        html.push_str("</head><body ");
        html.push_str(&sanitized_body_attrs);
        html.push('>');
    }

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

    // Patch <html> and <body> attributes to match the RSC flight data.
    // In the streaming path the head shell was flushed before V8 rendered,
    // so the attributes are not yet on the DOM. This sync script applies them
    // before React hydration runs, preventing hydration mismatches.
    let html_attrs = extract_html_tag_attrs(ssr_html);
    let body_attrs = extract_body_tag_attrs(ssr_html);
    if !html_attrs.is_empty() || !body_attrs.is_empty() {
        html.push_str("<script>");
        if !html_attrs.is_empty() {
            let escaped = escape_script_content(&html_attrs);
            html.push_str(&format!(
                "!function(){{var d=document.createElement('div');d.innerHTML='<span {escaped}>';var a=d.firstChild.attributes;for(var i=0;i<a.length;i++)document.documentElement.setAttribute(a[i].name,a[i].value)}}();"
            ));
        }
        if !body_attrs.is_empty() {
            let escaped = escape_script_content(&body_attrs);
            html.push_str(&format!(
                "!function(){{var d=document.createElement('div');d.innerHTML='<span {escaped}>';var a=d.firstChild.attributes;for(var i=0;i<a.length;i++)document.body.setAttribute(a[i].name,a[i].value)}}();"
            ));
        }
        html.push_str("</script>");
    }

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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn rsc_head_shell_emits_doctype_and_head() {
        let chunks = vec!["rsc/chunk-react-abc123.js".to_string()];
        let manifest = r#"{"refs":{}}"#;
        let html = assemble_rsc_head_shell(&chunks, manifest, &[], &HashMap::new());

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<html><head>"));
        assert!(html.contains("<meta charset=\"utf-8\" />"));
        assert!(html.contains("</head><body>"));
        // Module map and webpack shims in the head shell
        assert!(html.contains("__REX_RSC_MODULE_MAP__"));
        assert!(html.contains("__rexModuleCache"));
        // Modulepreload links in the head
        assert!(html.contains(
            r#"<link rel="modulepreload" href="/_rex/static/rsc/chunk-react-abc123.js" />"#
        ));
    }

    #[test]
    fn rsc_body_tail_extracts_body_content() {
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

        // Body content extracted from SSR HTML
        assert!(html.contains("<div>Hello</div>"));
        // Flight data present
        assert!(html.contains("__REX_RSC_DATA__"));
        assert!(html.contains("0:\"hello\""));
        // Script tags present
        assert!(html.contains(
            r#"<script type="module" src="/_rex/static/rsc/component-abc.js"></script>"#
        ));
        assert!(html.contains("</body></html>"));
    }

    #[test]
    fn rsc_body_tail_fallback_without_html_wrapper() {
        // SSR HTML without <body> tags: entire string used as body content
        let ssr = "<div>Hello</div>";
        let html = assemble_rsc_body_tail(ssr, "", "0:\"hi\"", &[], "{}", false, None);

        // Falls back to using the entire SSR HTML as body content
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

        // Body tail: body content → flight data → route manifest → component scripts → router → HMR
        let body_content_pos = html.find("<p>test</p>").unwrap();
        let flight_pos = html.find("__REX_RSC_DATA__").unwrap();
        let comp_script_pos = html
            .find(r#"<script type="module" src="/_rex/static/rsc/comp.js">"#)
            .unwrap();
        let router_pos = html.find("router.js").unwrap();
        let hmr_pos = html.find("hmr-client.js").unwrap();

        assert!(body_content_pos < flight_pos);
        assert!(flight_pos < comp_script_pos);
        assert!(comp_script_pos < router_pos);
        assert!(router_pos < hmr_pos);
    }

    #[test]
    fn rsc_head_shell_includes_module_map() {
        let manifest = r#"{"entries":{"abc":{"chunk_url":"/c.js","export_name":"default"}}}"#;
        let html = assemble_rsc_head_shell(&[], manifest, &[], &HashMap::new());

        assert!(html.contains("__REX_RSC_MODULE_MAP__"));
        assert!(html.contains("abc"));
    }

    #[test]
    fn rsc_head_shell_inlines_css() {
        let css_files = vec!["globals-abc12345.css".to_string()];
        let mut css_contents = HashMap::new();
        css_contents.insert(
            "globals-abc12345.css".to_string(),
            "body{margin:0}".to_string(),
        );
        let html = assemble_rsc_head_shell(&[], "{}", &css_files, &css_contents);

        assert!(html.contains("<style>body{margin:0}</style>"));
    }

    #[test]
    fn rsc_head_shell_falls_back_to_link_tag() {
        let css_files = vec!["missing.css".to_string()];
        let html = assemble_rsc_head_shell(&[], "{}", &css_files, &HashMap::new());

        assert!(html.contains(r#"<link rel="stylesheet" href="/_rex/static/missing.css" />"#));
    }

    #[test]
    fn rsc_body_tail_includes_metadata_head() {
        let ssr = "<html><head></head><body><div>Content</div></body></html>";
        let head_html = r#"<title>My Page</title><meta name="description" content="Test" />"#;
        let html = assemble_rsc_body_tail(ssr, head_html, "0:{}", &[], "{}", false, None);

        // Metadata head should appear before body content
        let title_pos = html.find("<title>My Page</title>").unwrap();
        let content_pos = html.find("<div>Content</div>").unwrap();
        assert!(
            title_pos < content_pos,
            "Metadata head should appear before body content"
        );
        assert!(html.contains(r#"<meta name="description" content="Test" />"#));
    }

    #[test]
    fn rsc_body_tail_empty_metadata_head() {
        let ssr = "<html><head></head><body><div>Content</div></body></html>";
        let html = assemble_rsc_body_tail(ssr, "", "0:{}", &[], "{}", false, None);

        // No metadata head and no attrs — body content should be first
        assert!(html.starts_with("<div>Content</div>"));
    }

    #[test]
    fn extract_html_tag_attrs_with_lang() {
        let ssr = r#"<html lang="en"><head></head><body>hi</body></html>"#;
        assert_eq!(extract_html_tag_attrs(ssr), r#"lang="en""#);
    }

    #[test]
    fn extract_html_tag_attrs_none() {
        let ssr = "<html><head></head><body>hi</body></html>";
        assert_eq!(extract_html_tag_attrs(ssr), "");
    }

    #[test]
    fn extract_body_tag_attrs_with_class() {
        let ssr = r#"<html><head></head><body class="bg-white">hi</body></html>"#;
        assert_eq!(extract_body_tag_attrs(ssr), r#"class="bg-white""#);
    }

    #[test]
    fn extract_body_tag_attrs_none() {
        let ssr = "<html><head></head><body>hi</body></html>";
        assert_eq!(extract_body_tag_attrs(ssr), "");
    }

    #[test]
    fn rsc_document_includes_html_and_body_attrs() {
        let params = RscDocumentParams {
            ssr_html: r#"<html lang="en"><head></head><body class="dark">Hello</body></html>"#,
            head_html: "",
            flight_data: "0:\"hi\"",
            client_chunks: &[],
            client_manifest_json: "{}",
            css_files: &[],
            css_contents: &HashMap::new(),
            is_dev: false,
            manifest_json: None,
            html_attrs: r#"lang="en""#,
            body_attrs: r#"class="dark""#,
        };
        let html = assemble_rsc_document(&params);
        assert!(html.contains(r#"<html lang="en"><head>"#));
        assert!(html.contains(r#"<body class="dark">"#));
    }

    #[test]
    fn sanitize_attrs_empty() {
        assert_eq!(sanitize_attrs(""), "");
    }

    #[test]
    fn sanitize_attrs_single_attribute() {
        assert_eq!(sanitize_attrs(r#"lang="en""#), r#"lang="en""#);
    }

    #[test]
    fn sanitize_attrs_multiple_attributes() {
        assert_eq!(
            sanitize_attrs(r#"lang="en" class="dark""#),
            r#"lang="en" class="dark""#
        );
    }

    #[test]
    fn sanitize_attrs_single_quotes_converted() {
        // Single quotes should be converted to double quotes
        assert_eq!(sanitize_attrs(r#"lang='en'"#), r#"lang="en""#);
    }

    #[test]
    fn sanitize_attrs_escapes_angle_brackets() {
        // Angle brackets in attribute values should be escaped
        assert_eq!(
            sanitize_attrs(r#"data="<script>""#),
            r#"data="&lt;script&gt;""#
        );
    }

    #[test]
    fn sanitize_attrs_escapes_quotes_in_value() {
        // Double quotes inside value should be escaped
        assert_eq!(
            sanitize_attrs(r#"data='say "hello"'"#),
            r#"data="say &quot;hello&quot;""#
        );
    }

    #[test]
    fn sanitize_attrs_xss_injection_attempt() {
        // Classic XSS injection attempt: break out of attribute, inject HTML
        let malicious = r#"lang='"><img src=x onerror=alert(1) a="'"#;
        let result = sanitize_attrs(malicious);
        // The dangerous characters should be escaped
        assert!(result.contains("&quot;"));
        assert!(result.contains("&gt;"));
        assert!(result.contains("&lt;"));
        // No raw angle brackets or quotes that would allow breakout
        assert!(!result.contains("\"><"));
        assert!(!result.contains("<img"));
    }

    #[test]
    fn sanitize_attrs_boolean_attribute() {
        // Boolean attributes without a value should be preserved
        assert_eq!(sanitize_attrs("disabled"), "disabled");
        assert_eq!(
            sanitize_attrs(r#"disabled class="btn""#),
            r#"disabled class="btn""#
        );
    }

    #[test]
    fn sanitize_attrs_unquoted_value() {
        // Unquoted attribute values should be quoted in output
        assert_eq!(sanitize_attrs("lang=en"), r#"lang="en""#);
    }

    #[test]
    fn sanitize_attrs_ampersand_escaped() {
        assert_eq!(
            sanitize_attrs(r#"data="a&b""#),
            r#"data="a&amp;b""#
        );
    }

    #[test]
    fn sanitize_attrs_preserves_data_attributes() {
        assert_eq!(
            sanitize_attrs(r#"data-testid="my-test" data-value="123""#),
            r#"data-testid="my-test" data-value="123""#
        );
    }

    #[test]
    fn rsc_head_shell_sanitizes_malicious_html_attrs() {
        let malicious_html_attrs = r#"lang='"><script>alert(1)</script><span a="'"#;
        let html = assemble_rsc_head_shell_with_attrs(
            &[],
            "{}",
            &[],
            &HashMap::new(),
            malicious_html_attrs,
            "",
        );
        // Should not contain unescaped injection
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(!html.contains("><script>"));
        // Should contain escaped version
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn rsc_head_shell_sanitizes_malicious_body_attrs() {
        let malicious_body_attrs = r#"class='"><img src=x onerror=alert(1)>'"#;
        let html = assemble_rsc_head_shell_with_attrs(
            &[],
            "{}",
            &[],
            &HashMap::new(),
            "",
            malicious_body_attrs,
        );
        // Should not contain unescaped injection
        assert!(!html.contains("<img src=x onerror=alert(1)>"));
        assert!(!html.contains("><img"));
        // Should contain escaped version
        assert!(html.contains("&lt;img"));
    }
}
