/// Assemble the final HTML document
pub fn assemble_document(
    ssr_html: &str,
    props_json: &str,
    vendor_scripts: &[String],
    client_scripts: &[String],
    _build_id: &str,
    is_dev: bool,
) -> String {
    let escaped_props = escape_script_content(props_json);

    let mut html = String::with_capacity(ssr_html.len() + 2048);

    html.push_str("<!DOCTYPE html>\n");
    html.push_str("<html>\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\" />\n");
    html.push_str("  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n");

    // Preload client chunks
    for script in client_scripts {
        html.push_str(&format!(
            "  <link rel=\"modulepreload\" href=\"/_rex/static/{script}\" />\n"
        ));
    }

    html.push_str("</head>\n<body>\n");

    // Main content
    html.push_str(&format!("  <div id=\"__rex\">{ssr_html}</div>\n"));

    // Props data for hydration
    html.push_str(&format!(
        "  <script id=\"__REX_DATA__\" type=\"application/json\">{escaped_props}</script>\n"
    ));

    // React vendor scripts (built from node_modules at build time)
    for script in vendor_scripts {
        html.push_str(&format!(
            "  <script src=\"/_rex/static/{script}\"></script>\n"
        ));
    }

    // Client chunks
    for script in client_scripts {
        html.push_str(&format!(
            "  <script src=\"/_rex/static/{script}\"></script>\n"
        ));
    }

    // HMR client in dev mode
    if is_dev {
        html.push_str("  <script src=\"/_rex/hmr-client.js\"></script>\n");
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
