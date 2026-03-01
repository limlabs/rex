use crate::cookies::cookies_from_header_map;
use crate::csrf;
use crate::session::{self, SessionData};
use crate::AuthServer;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;

/// GET /_rex/auth/signin?provider=github&callbackUrl=/dashboard
pub async fn signin_handler(
    State(auth): State<Arc<AuthServer>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let provider_id = match params.get("provider") {
        Some(id) => id,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Missing provider parameter",
            )
                .into_response();
        }
    };

    let provider = match auth.providers.get(provider_id.as_str()) {
        Some(p) => p,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                format!("Unknown provider: {provider_id}"),
            )
                .into_response();
        }
    };

    // Validate callbackUrl — must be relative or same-origin
    if let Some(callback_url) = params.get("callbackUrl") {
        if !is_safe_callback_url(callback_url) {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid callbackUrl").into_response();
        }
    }

    // Generate CSRF state
    let state = csrf::generate_state();
    let callback_url = format!("{}/_rex/auth/callback/{}", auth.base_url, provider_id);

    let auth_url = provider.authorization_url(&state, &callback_url);

    // Preserve callbackUrl in a separate cookie for the post-login redirect
    let post_login_redirect = params
        .get("callbackUrl")
        .filter(|u| is_safe_callback_url(u))
        .cloned();
    let csrf_cookie = csrf::csrf_state_cookie(&state, auth.is_dev);

    let mut builder = Response::builder()
        .status(302)
        .header("Location", auth_url)
        .header("Set-Cookie", csrf_cookie);

    if let Some(ref redirect) = post_login_redirect {
        builder = builder.header(
            "Set-Cookie",
            csrf::callback_url_cookie(redirect, auth.is_dev),
        );
    }

    builder.body(axum::body::Body::empty()).unwrap_or_else(|_| {
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Error").into_response()
    })
}

/// GET /_rex/auth/callback/{provider}?code=xxx&state=yyy
pub async fn callback_handler(
    State(auth): State<Arc<AuthServer>>,
    Path(provider_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Validate provider ID — reject path traversal
    if provider_id.contains('.')
        || provider_id.contains('/')
        || provider_id.contains('\\')
        || provider_id.contains('%')
    {
        return (axum::http::StatusCode::BAD_REQUEST, "Invalid provider").into_response();
    }

    let provider = match auth.providers.get(provider_id.as_str()) {
        Some(p) => p,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                format!("Unknown provider: {provider_id}"),
            )
                .into_response();
        }
    };

    // Validate CSRF state
    let received_state = match params.get("state") {
        Some(s) => s,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Missing state parameter",
            )
                .into_response();
        }
    };

    let cookies = cookies_from_header_map(&headers);

    let expected_state = match cookies.get("__rex_auth_state") {
        Some(s) => s,
        None => {
            return (axum::http::StatusCode::BAD_REQUEST, "Missing CSRF cookie").into_response();
        }
    };

    if !csrf::validate_state(received_state, expected_state) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "CSRF validation failed",
        )
            .into_response();
    }

    // Check for error from provider
    if let Some(error) = params.get("error") {
        let desc = params
            .get("error_description")
            .map(|s| s.as_str())
            .unwrap_or("Unknown error");
        tracing::error!(provider = %provider_id, error = %error, desc = %desc, "OAuth error");
        return (
            axum::http::StatusCode::BAD_REQUEST,
            format!("OAuth error: {desc}"),
        )
            .into_response();
    }

    // Exchange code for tokens
    let code = match params.get("code") {
        Some(c) => c,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Missing code parameter",
            )
                .into_response();
        }
    };

    let callback_url = format!("{}/_rex/auth/callback/{}", auth.base_url, provider_id);

    // Compute PKCE code_verifier if the provider uses PKCE
    let verifier = provider.code_verifier(received_state);
    let tokens = match provider
        .exchange_code(code, &callback_url, &auth.http_client, verifier.as_deref())
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Token exchange error: {e}");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Token exchange failed",
            )
                .into_response();
        }
    };

    // Fetch user profile
    let user = match provider
        .fetch_user_profile(&tokens, &auth.http_client)
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("User profile fetch error: {e}");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to fetch user profile",
            )
                .into_response();
        }
    };

    // Create session
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + auth.config.session.max_age;

    let session_data = SessionData {
        user,
        provider: provider_id.clone(),
        access_token: Some(tokens.access_token.clone()),
        expires,
    };

    let encrypted = match session::encrypt_session(&session_data, &auth.session_key) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Session encryption error: {e}");
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Session creation failed",
            )
                .into_response();
        }
    };

    let session_cookie = session::session_cookie(
        &auth.config.session.cookie_name,
        &encrypted,
        auth.config.session.max_age,
        auth.is_dev,
    );
    let clear_csrf = csrf::clear_csrf_cookie(auth.is_dev);
    let clear_callback = csrf::clear_callback_url_cookie(auth.is_dev);

    // Redirect to callback URL (stored in cookie during signin) or home
    let redirect_to = cookies
        .get("__rex_callback_url")
        .filter(|u| is_safe_callback_url(u))
        .map(|s| s.as_str())
        .unwrap_or("/");

    Response::builder()
        .status(302)
        .header("Location", redirect_to)
        .header("Set-Cookie", session_cookie)
        .header("Set-Cookie", clear_csrf)
        .header("Set-Cookie", clear_callback)
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Error").into_response()
        })
}

/// POST /_rex/auth/signout
pub async fn signout_handler(State(auth): State<Arc<AuthServer>>) -> Response {
    let clear_cookie = session::clear_session_cookie(&auth.config.session.cookie_name, auth.is_dev);

    Response::builder()
        .status(302)
        .header("Location", "/")
        .header("Set-Cookie", clear_cookie)
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| {
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Error").into_response()
        })
}

/// GET /_rex/auth/session
pub async fn session_handler(
    State(auth): State<Arc<AuthServer>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let cookies = cookies_from_header_map(&headers);

    let session = cookies
        .get(&auth.config.session.cookie_name)
        .and_then(|encrypted| session::decrypt_session(encrypted, &auth.session_key));

    match session {
        Some(data) => {
            let body = serde_json::json!({
                "user": data.user,
                "expires": data.expires,
            });
            (
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
            )
                .into_response()
        }
        None => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            r#"{"user":null,"status":"unauthenticated"}"#,
        )
            .into_response(),
    }
}

/// Extract session from request headers (for GSSP context).
pub fn extract_session(
    headers: &HashMap<String, String>,
    session_key: &[u8; 32],
    cookie_name: &str,
) -> Option<serde_json::Value> {
    let cookies = crate::cookies::cookies_from_headers(headers);
    let encrypted = cookies.get(cookie_name)?;
    let data = session::decrypt_session(encrypted, session_key)?;
    serde_json::to_value(&data).ok()
}

/// Validate that a callback URL is safe (relative path or same origin).
fn is_safe_callback_url(url: &str) -> bool {
    // Must start with / (relative path)
    if !url.starts_with('/') {
        return false;
    }
    // Must not be protocol-relative (//evil.com)
    if url.starts_with("//") {
        return false;
    }
    // Must not contain line breaks (header injection)
    if url.contains('\r') || url.contains('\n') {
        return false;
    }
    // Must not be a data: or javascript: URI
    let lower = url.to_lowercase();
    if lower.starts_with("/data:") || lower.starts_with("/javascript:") {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_callback_url() {
        assert!(is_safe_callback_url("/"));
        assert!(is_safe_callback_url("/dashboard"));
        assert!(is_safe_callback_url("/auth/callback?foo=bar"));
    }

    #[test]
    fn test_unsafe_callback_urls() {
        assert!(!is_safe_callback_url("https://evil.com"));
        assert!(!is_safe_callback_url("//evil.com/steal"));
        assert!(!is_safe_callback_url("http://evil.com"));
        assert!(!is_safe_callback_url("data:text/html,<script>"));
        assert!(!is_safe_callback_url("/test\r\nSet-Cookie: evil=true"));
    }
}
