#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn escape_script_content_script_tag() {
    let input = r#"var x = "</script>";"#;
    let result = escape_script_content(input);
    assert!(!result.contains("</script"));
    assert!(!result.contains('<'));
}

#[test]
fn escape_script_content_case_insensitive() {
    // HTML parsers match end tags case-insensitively, so </SCRIPT> and
    // </Script> must also be neutralized.
    for tag in ["</SCRIPT>", "</Script>", "</sCrIpT>"] {
        let input = format!(r#"{{"x": "{tag}"}}"#);
        let result = escape_script_content(&input);
        assert!(
            !result.contains('<'),
            "escape_script_content must neutralize {tag}"
        );
    }
}

#[test]
fn escape_script_content_html_comment() {
    let input = "<!-- comment -->";
    let result = escape_script_content(input);
    assert!(!result.contains("<!--"));
    assert!(!result.contains('<'));
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
fn escape_style_content_case_insensitive() {
    for tag in ["</style>", "</STYLE>", "</Style>"] {
        let input = format!("body{{}} {tag}");
        let result = escape_style_content(&input);
        assert!(
            !result.contains('<'),
            "escape_style_content must neutralize {tag}"
        );
    }
}

#[test]
fn assemble_document_escapes_inline_css() {
    let mut css_contents = HashMap::new();
    css_contents.insert(
        "test.css".to_string(),
        "body{} </style><script>alert(1)</script>".to_string(),
    );
    let params = DocumentParams {
        ssr_html: "<div>hi</div>",
        head_html: "",
        props_json: "{}",
        client_scripts: &[],
        css_files: &["test.css".to_string()],
        css_contents: &css_contents,
        app_script: None,
        is_dev: false,
        doc_descriptor: None,
        manifest_json: None,
        font_preloads: &[],
        import_map_json: None,
    };
    let html = assemble_document(&params);
    // The </style> inside the CSS must be escaped so the style block isn't closed prematurely
    assert!(!html.contains("</style><script>"));
}

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
    assert!(html
        .contains(r#"<link rel="modulepreload" href="/_rex/static/rsc/chunk-react-abc123.js" />"#));
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
    assert!(
        html.contains(r#"<script type="module" src="/_rex/static/rsc/component-abc.js"></script>"#)
    );
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
