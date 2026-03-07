mod action;
mod api;
mod data;
mod image;
mod page;
mod rsc;

pub use action::server_action_handler;
pub use api::api_handler;
pub use data::data_handler;
pub use image::{image_handler, ImageQuery};
pub use page::page_handler;
pub use rsc::rsc_handler;

pub use crate::state::{AppState, HotState};

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use rex_core::ProjectConfig;
use std::sync::Arc;
use tracing::debug;

use crate::document::{assemble_document, DocumentDescriptor, DocumentParams};

pub(crate) use crate::core::{
    check_rewrites, collect_custom_headers as collect_headers, execute_middleware,
    should_run_middleware,
};

/// Generate a full-page error overlay for dev mode.
/// Includes HMR WebSocket connection for auto-reload on fix.
pub(crate) fn dev_error_overlay(title: &str, message: &str, file: Option<&str>) -> String {
    let escaped_message = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let file_section = file
        .map(|f| {
            let escaped = f
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            format!(r#"<div class="file">{escaped}</div>"#)
        })
        .unwrap_or_default();

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
  background: #1a1a2e;
  color: #e0e0e0;
  font-family: 'SF Mono', 'Fira Code', 'JetBrains Mono', Menlo, Consolas, monospace;
  min-height: 100vh;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 60px 20px;
}}
.container {{
  max-width: 860px;
  width: 100%;
}}
.badge {{
  display: inline-block;
  background: #e63946;
  color: #fff;
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding: 4px 10px;
  border-radius: 4px;
  margin-bottom: 16px;
}}
h1 {{
  font-size: 22px;
  font-weight: 600;
  color: #fff;
  margin-bottom: 20px;
  line-height: 1.4;
}}
.file {{
  color: #8892b0;
  font-size: 13px;
  margin-bottom: 16px;
  padding: 8px 12px;
  background: rgba(255,255,255,0.04);
  border-radius: 6px;
  border-left: 3px solid #e63946;
}}
.stack {{
  background: #0d1117;
  border: 1px solid rgba(255,255,255,0.08);
  border-radius: 8px;
  padding: 20px;
  overflow-x: auto;
  font-size: 13px;
  line-height: 1.7;
  white-space: pre-wrap;
  word-wrap: break-word;
  color: #f0c674;
}}
.hint {{
  margin-top: 24px;
  font-size: 12px;
  color: #555;
}}
.dot {{
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 8px;
  background: #e63946;
  animation: pulse 2s infinite;
}}
.dot.connected {{ background: #2ecc71; animation: none; }}
@keyframes pulse {{ 0%,100% {{ opacity: 1; }} 50% {{ opacity: 0.3; }} }}
</style>
</head>
<body>
<div class="container">
  <div class="badge">{title}</div>
  {file_section}
  <div class="stack">{escaped_message}</div>
  <div class="hint"><span class="dot" id="dot"></span><span id="status">Waiting for changes...</span></div>
</div>
<script>
(function() {{
  var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  var ws = new WebSocket(proto + '//' + location.host + '/_rex/hmr');
  var dot = document.getElementById('dot');
  var status = document.getElementById('status');
  ws.onopen = function() {{
    dot.className = 'dot connected';
    status.textContent = 'Connected — save a file to reload';
  }};
  ws.onmessage = function(e) {{
    try {{ var m = JSON.parse(e.data); }} catch(x) {{ return; }}
    if (m.type === 'update' || m.type === 'full-reload') location.reload();
  }};
  ws.onclose = function() {{
    dot.className = 'dot';
    status.textContent = 'Disconnected — retrying...';
    setTimeout(function() {{ location.reload(); }}, 2000);
  }};
}})();
</script>
</body>
</html>"#
    )
}

/// Render a custom error page (404 or _error) via SSR, returning the full HTML document.
pub(crate) async fn render_error_page(
    state: &Arc<AppState>,
    hot: &Arc<HotState>,
    page_key: &str,
    status: StatusCode,
    props: &str,
) -> Response {
    let key = page_key.to_string();
    let props_clone = props.to_string();
    let ssr_result = state
        .isolate_pool
        .execute(move |iso| iso.render_page(&key, &props_clone))
        .await;

    let render = match ssr_result {
        Ok(Ok(r)) => r,
        _ => return (status, Html(format!("{} Error", status.as_u16()))).into_response(),
    };

    let document = assemble_document(&DocumentParams {
        ssr_html: &render.body,
        head_html: &render.head,
        props_json: props,
        client_scripts: &[],
        css_files: &hot.manifest.global_css,
        css_contents: &hot.manifest.css_contents,
        app_script: hot.manifest.app_script.as_deref(),
        is_dev: state.is_dev,
        doc_descriptor: hot.document_descriptor.as_ref(),
        manifest_json: Some(&hot.manifest_json),
    });

    (status, Html(document)).into_response()
}

/// Compute document descriptor from V8 _document rendering.
/// Used to populate `HotState.document_descriptor` at build time and on rebuilds.
pub async fn compute_document_descriptor(pool: &rex_v8::IsolatePool) -> Option<DocumentDescriptor> {
    let result = pool.execute(move |iso| iso.render_document()).await;
    match result {
        Ok(Ok(Some(json))) => serde_json::from_str(&json).ok(),
        _ => None,
    }
}

/// Check redirect rules and return early if matched.
pub(crate) fn check_redirects(path: &str, config: &ProjectConfig) -> Option<Response> {
    for rule in &config.redirects {
        if let Some(params) = ProjectConfig::match_pattern(&rule.source, path) {
            let dest = ProjectConfig::apply_params(&rule.destination, &params);
            let status = if rule.permanent {
                StatusCode::PERMANENT_REDIRECT // 308
            } else {
                StatusCode::from_u16(rule.status_code).unwrap_or(StatusCode::TEMPORARY_REDIRECT)
            };
            debug!(from = path, to = %dest, status = %status.as_u16(), "Redirect rule matched");
            return Some(
                Response::builder()
                    .status(status)
                    .header("location", &dest)
                    .body(Body::empty())
                    .expect("response build"),
            );
        }
    }
    None
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_dev_error_overlay_escapes_html() {
        let overlay = dev_error_overlay("Test Error", "<script>alert('xss')</script>", None);
        assert!(overlay.contains("&lt;script&gt;"));
        assert!(!overlay.contains("<script>alert"));
        assert!(overlay.contains("Test Error"));
    }

    #[test]
    fn test_dev_error_overlay_with_file_section() {
        let overlay = dev_error_overlay("Build Error", "some error", Some("pages/index.tsx"));
        assert!(overlay.contains("pages/index.tsx"));
        assert!(overlay.contains("Build Error"));
    }

    #[test]
    fn test_dev_error_overlay_hmr_script() {
        let overlay = dev_error_overlay("Error", "msg", None);
        assert!(
            overlay.contains("/_rex/hmr"),
            "should include HMR WebSocket"
        );
        assert!(
            overlay.contains("WebSocket"),
            "should include WebSocket reconnect"
        );
    }

    #[test]
    fn test_check_redirects_no_match() {
        let config = rex_core::ProjectConfig::default();
        assert!(check_redirects("/anything", &config).is_none());
    }

    #[test]
    fn test_check_redirects_match() {
        let config = rex_core::ProjectConfig {
            redirects: vec![rex_core::RedirectRule {
                source: "/old".to_string(),
                destination: "/new".to_string(),
                status_code: 301,
                permanent: false,
            }],
            ..Default::default()
        };
        let resp = check_redirects("/old", &config).unwrap();
        assert_eq!(resp.status(), 301);
    }
}
