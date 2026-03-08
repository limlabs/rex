use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

use super::AppState;
use crate::state::snapshot;

/// Maximum body size for server action requests (1 MB).
const MAX_ACTION_BODY_SIZE: usize = 1024 * 1024;

/// Server action handler: POST /_rex/action/{build_id}/{action_id}
///
/// Dispatches a server function call from the client. Supports three content types:
/// - `application/json`: Legacy path — body is a JSON array of arguments
/// - `text/x-component`: Encoded reply — body is a string from React's `encodeReply`
/// - `multipart/form-data`: Form submission — body is parsed multipart fields
///
/// Returns flight data (`text/x-component`) for success, or JSON `{ error }` for errors.
/// Special headers for redirect/notFound: `X-Rex-Redirect`, `X-Rex-Not-Found`.
pub async fn server_action_handler(
    State(state): State<Arc<AppState>>,
    Path((build_id, action_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let hot = snapshot(&state);

    // Build ID mismatch = stale client
    if build_id != hot.build_id {
        return StatusCode::NOT_FOUND.into_response();
    }

    // CSRF protection: validate Origin header against Host
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
            let origin_host = origin
                .trim_start_matches("https://")
                .trim_start_matches("http://");
            if origin_host != host {
                return (StatusCode::FORBIDDEN, "CSRF validation failed").into_response();
            }
        }
    }

    // Body size limit
    if body.len() > MAX_ACTION_BODY_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response();
    }

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    // Serialize request context for V8
    let header_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let headers_json = serde_json::to_string(&header_map).unwrap_or_else(|_| "{}".to_string());
    let cookies: HashMap<String, String> = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .map(|cookie_str| {
            cookie_str
                .split(';')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    Some((
                        parts.next()?.trim().to_string(),
                        parts.next()?.trim().to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let cookies_json = serde_json::to_string(&cookies).unwrap_or_else(|_| "{}".to_string());

    let result = if content_type.starts_with("text/x-component") {
        // Encoded reply path: React's encodeReply produced a string
        let body_str = match std::str::from_utf8(&body) {
            Ok(s) => s.to_string(),
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid UTF-8 body").into_response();
            }
        };
        let action_id_owned = action_id.clone();
        state
            .isolate_pool
            .execute(move |iso| {
                let _ = iso.set_request_context(&headers_json, &cookies_json);
                let r = iso.call_server_action_encoded(&action_id_owned, &body_str, false);
                let _ = iso.clear_request_context();
                r
            })
            .await
    } else if content_type.starts_with("multipart/form-data") {
        // Multipart from callServer: encodeReply returned FormData for complex args (Blob, File).
        // Use decodeReply (not decodeAction) since the action ID is in the URL, not the FormData.
        let boundary = content_type
            .split("boundary=")
            .nth(1)
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .to_string();
        let fields = parse_multipart_fields(&body, &boundary);
        let fields_json = serde_json::to_string(&fields).unwrap_or_else(|_| "[]".to_string());
        let action_id_owned = action_id.clone();
        state
            .isolate_pool
            .execute(move |iso| {
                let _ = iso.set_request_context(&headers_json, &cookies_json);
                let r = iso.call_server_action_encoded(&action_id_owned, &fields_json, true);
                let _ = iso.clear_request_context();
                r
            })
            .await
    } else {
        // Legacy JSON path
        let args_json = match std::str::from_utf8(&body) {
            Ok(s) => s.to_string(),
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "Invalid UTF-8 body").into_response();
            }
        };
        let action_id_owned = action_id.clone();
        state
            .isolate_pool
            .execute(move |iso| {
                let _ = iso.set_request_context(&headers_json, &cookies_json);
                let r = iso.call_server_action(&action_id_owned, &args_json);
                let _ = iso.clear_request_context();
                r
            })
            .await
    };

    match result {
        Ok(Ok(json_result)) => build_action_response(&json_result),
        Ok(Err(e)) => {
            error!("Server action error: {e}");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "error": e.to_string() }).to_string(),
                ))
                .expect("response build")
        }
        Err(e) => {
            error!("Server action pool error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Parse the JSON envelope from `__rex_finalize_action()` and build the HTTP response.
fn build_action_response(json_result: &str) -> Response {
    let parsed: serde_json::Value = match serde_json::from_str(json_result) {
        Ok(v) => v,
        Err(_) => {
            return Response::builder()
                .header("Content-Type", "application/json")
                .body(Body::from(json_result.to_string()))
                .expect("response build");
        }
    };

    // Redirect
    if let Some(redirect_url) = parsed.get("redirect").and_then(|v| v.as_str()) {
        let status = parsed
            .get("redirectStatus")
            .and_then(|v| v.as_u64())
            .unwrap_or(303) as u16;
        return Response::builder()
            .status(StatusCode::from_u16(status).unwrap_or(StatusCode::SEE_OTHER))
            .header("Location", redirect_url)
            .header("X-Rex-Redirect", redirect_url)
            .body(Body::empty())
            .expect("response build");
    }

    // Not found
    if parsed
        .get("notFound")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("X-Rex-Not-Found", "1")
            .body(Body::empty())
            .expect("response build");
    }

    // Flight data (success)
    if let Some(flight) = parsed.get("flight").and_then(|v| v.as_str()) {
        return Response::builder()
            .header("Content-Type", "text/x-component")
            .header("Cache-Control", "no-cache")
            .body(Body::from(flight.to_string()))
            .expect("response build");
    }

    // Error
    if parsed.get("error").is_some() {
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("Content-Type", "application/json")
            .body(Body::from(json_result.to_string()))
            .expect("response build");
    }

    // Legacy JSON result fallback
    Response::builder()
        .header("Content-Type", "application/json")
        .body(Body::from(json_result.to_string()))
        .expect("response build")
}

/// Parse multipart/form-data fields into a Vec of (key, value) pairs.
/// Only handles text fields (not file uploads) — sufficient for React form actions.
pub(super) fn parse_multipart_fields(body: &[u8], boundary: &str) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let body_str = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return fields,
    };

    let delimiter = format!("--{boundary}");
    let parts: Vec<&str> = body_str.split(&delimiter).collect();

    for part in parts {
        let part = part.trim_start_matches("\r\n");
        if part.is_empty() || part == "--" || part == "--\r\n" {
            continue;
        }

        // Split headers from body at double CRLF
        if let Some(header_end) = part.find("\r\n\r\n") {
            let headers_section = &part[..header_end];
            let value = part[header_end + 4..].trim_end_matches("\r\n");

            // Extract name from Content-Disposition header
            if let Some(name) = extract_field_name(headers_section) {
                fields.push((name, value.to_string()));
            }
        }
    }

    fields
}

/// Extract the `name` attribute from a Content-Disposition header.
fn extract_field_name(headers: &str) -> Option<String> {
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("content-disposition") {
            if let Some(pos) = lower.find("; name=\"") {
                let name_start = pos + 8;
                let rest = &line[name_start..];
                if let Some(name_end) = rest.find('"') {
                    return Some(rest[..name_end].to_string());
                }
            }
        }
    }
    None
}

/// Parse form fields from a POST request body for progressive enhancement.
/// Returns Some(fields) for multipart/form-data or application/x-www-form-urlencoded,
/// None for other content types.
pub(super) fn parse_form_action_fields(
    headers: &HeaderMap,
    body: &[u8],
) -> Option<Vec<(String, String)>> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type.starts_with("multipart/form-data") {
        let boundary = content_type
            .split("boundary=")
            .nth(1)
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        Some(parse_multipart_fields(body, boundary))
    } else if content_type.starts_with("application/x-www-form-urlencoded") {
        Some(
            url::form_urlencoded::parse(body)
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::handlers::test_support::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_server_action_stale_build_id() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/wrong-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .body(Body::from("[42]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_server_action_success() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .body(Body::from("[42]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp.into_body()).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["result"], 43);
    }

    #[tokio::test]
    async fn test_server_action_not_found() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/nonexistent")
                    .header("Content-Type", "application/json")
                    .body(Body::from("[]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_string(resp.into_body()).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].as_str().unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_server_action_invalid_utf8() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .body(Body::from(vec![0xFF, 0xFE]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_server_action_csrf_rejection() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .header("Origin", "https://evil.com")
                    .header("Host", "localhost:3000")
                    .body(Body::from("[42]"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_server_action_payload_too_large() {
        let app = TestAppBuilder::new()
            .extra_bundle(TEST_ACTION_RUNTIME)
            .custom_router(|state| {
                Router::new()
                    .route(
                        "/_rex/action/{build_id}/{action_id}",
                        post(server_action_handler),
                    )
                    .with_state(state)
            })
            .build();

        let huge_body = vec![b'x'; MAX_ACTION_BODY_SIZE + 1];
        let resp = app
            .oneshot(
                Request::post("/_rex/action/test-build-id/test_action_id")
                    .header("Content-Type", "application/json")
                    .body(Body::from(huge_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn test_build_action_response_redirect() {
        let resp = build_action_response(r#"{"redirect":"/dashboard","redirectStatus":303}"#);
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(resp.headers().get("Location").unwrap(), "/dashboard");
        assert_eq!(resp.headers().get("X-Rex-Redirect").unwrap(), "/dashboard");
    }

    #[test]
    fn test_build_action_response_not_found() {
        let resp = build_action_response(r#"{"notFound":true}"#);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(resp.headers().get("X-Rex-Not-Found").unwrap(), "1");
    }

    #[test]
    fn test_build_action_response_flight() {
        let resp = build_action_response(r#"{"flight":"0:\"hello\"\n"}"#);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("Content-Type").unwrap(),
            "text/x-component"
        );
    }

    #[test]
    fn test_build_action_response_error() {
        let resp = build_action_response(r#"{"error":"something failed"}"#);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_build_action_response_invalid_json() {
        let resp = build_action_response("not json at all");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_build_action_response_legacy_json() {
        let resp = build_action_response(r#"{"result":42}"#);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("Content-Type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_parse_multipart_fields_basic() {
        let body = "--boundary\r\nContent-Disposition: form-data; name=\"field1\"\r\n\r\nvalue1\r\n--boundary\r\nContent-Disposition: form-data; name=\"field2\"\r\n\r\nvalue2\r\n--boundary--\r\n";
        let fields = parse_multipart_fields(body.as_bytes(), "boundary");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], ("field1".to_string(), "value1".to_string()));
        assert_eq!(fields[1], ("field2".to_string(), "value2".to_string()));
    }

    #[test]
    fn test_parse_multipart_fields_empty() {
        let fields = parse_multipart_fields(b"--boundary--\r\n", "boundary");
        assert!(fields.is_empty());
    }

    #[test]
    fn test_parse_multipart_fields_invalid_utf8() {
        let fields = parse_multipart_fields(&[0xFF, 0xFE], "boundary");
        assert!(fields.is_empty());
    }

    #[test]
    fn test_extract_field_name_found() {
        let headers = "Content-Disposition: form-data; name=\"myfield\"";
        assert_eq!(extract_field_name(headers), Some("myfield".to_string()));
    }

    #[test]
    fn test_extract_field_name_not_found() {
        assert_eq!(extract_field_name("Content-Type: text/plain"), None);
    }

    #[test]
    fn test_parse_form_action_fields_urlencoded() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let body = b"key1=val1&key2=val2";
        let fields = parse_form_action_fields(&headers, body).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0], ("key1".to_string(), "val1".to_string()));
    }

    #[test]
    fn test_parse_form_action_fields_unsupported_content_type() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "text/plain".parse().unwrap());
        assert!(parse_form_action_fields(&headers, b"hello").is_none());
    }

    #[test]
    fn test_parse_form_action_fields_multipart() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            "multipart/form-data; boundary=abc".parse().unwrap(),
        );
        let body = "--abc\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\ny\r\n--abc--\r\n";
        let fields = parse_form_action_fields(&headers, body.as_bytes()).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], ("x".to_string(), "y".to_string()));
    }
}
