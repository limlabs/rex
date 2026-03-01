use crate::cookies::parse_cookies;
use crate::session;
use crate::AuthServer;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;

/// GET /_rex/auth/authorize — Authorization endpoint.
///
/// Validates the request, checks if the user is authenticated,
/// and shows a consent page or auto-approves.
pub async fn authorize_get_handler(
    State(auth): State<Arc<AuthServer>>,
    Query(params): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Validate required params
    let response_type = params.get("response_type").map(|s| s.as_str());
    if response_type != Some("code") {
        return error_response(
            400,
            "unsupported_response_type",
            "Only response_type=code is supported",
        );
    }

    let client_id = match params.get("client_id") {
        Some(id) => id.clone(),
        None => return error_response(400, "invalid_request", "Missing client_id"),
    };

    let redirect_uri = match params.get("redirect_uri") {
        Some(uri) => uri.clone(),
        None => return error_response(400, "invalid_request", "Missing redirect_uri"),
    };

    // PKCE is required (OAuth 2.1)
    let code_challenge = match params.get("code_challenge") {
        Some(c) => c.clone(),
        None => {
            return error_response(
                400,
                "invalid_request",
                "PKCE code_challenge is required (OAuth 2.1)",
            );
        }
    };

    let challenge_method = params
        .get("code_challenge_method")
        .map(|s| s.as_str())
        .unwrap_or("S256");
    if challenge_method != "S256" {
        return error_response(
            400,
            "invalid_request",
            "Only code_challenge_method=S256 is supported",
        );
    }

    let state = params.get("state").cloned().unwrap_or_default();
    let scope = params.get("scope").cloned().unwrap_or_default();

    // Validate client
    let store = match &auth.store {
        Some(s) => s,
        None => return error_response(500, "server_error", "Auth store not initialized"),
    };

    let client = match store.get_client(&client_id) {
        Ok(c) => c,
        Err(crate::AuthError::ClientNotFound(_)) => {
            return error_response(400, "invalid_client", "Unknown client_id");
        }
        Err(e) => {
            tracing::error!("Store error: {e}");
            return error_response(500, "server_error", "Internal error");
        }
    };

    // Validate redirect_uri against registered URIs
    if !validate_redirect_uri(&redirect_uri, &client.redirect_uris) {
        return error_response(400, "invalid_request", "redirect_uri not registered");
    }

    // Validate scopes
    let requested_scopes = crate::scopes::parse_scopes(&scope);
    let valid_scopes = crate::scopes::validate_scopes(&requested_scopes, &auth.config.mcp.scopes);

    // Check if user is authenticated (session cookie)
    let cookie_header = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let cookies = parse_cookies(cookie_header);

    let session_data = cookies
        .get(&auth.config.session.cookie_name)
        .and_then(|encrypted| session::decrypt_session(encrypted, &auth.session_key));

    let subject = match session_data {
        Some(ref data) => data.user.id.clone(),
        None => {
            // No OAuth providers configured → "owner" mode (MCP-only)
            if auth.providers.is_empty() {
                "owner".to_string()
            } else {
                // Redirect to sign-in with return URL
                let return_url = format!(
                    "/_rex/auth/authorize?{}",
                    url::form_urlencoded::Serializer::new(String::new())
                        .append_pair("response_type", "code")
                        .append_pair("client_id", &client_id)
                        .append_pair("redirect_uri", &redirect_uri)
                        .append_pair("code_challenge", &code_challenge)
                        .append_pair("code_challenge_method", "S256")
                        .append_pair("scope", &scope)
                        .append_pair("state", &state)
                        .finish()
                );
                let signin_url = auth
                    .config
                    .pages
                    .sign_in
                    .as_deref()
                    .unwrap_or("/_rex/auth/signin");
                let redirect = format!(
                    "{signin_url}?callbackUrl={}",
                    url::form_urlencoded::byte_serialize(return_url.as_bytes()).collect::<String>()
                );
                return Response::builder()
                    .status(302)
                    .header("Location", redirect)
                    .body(axum::body::Body::empty())
                    .unwrap_or_else(|_| error_response(500, "server_error", "Redirect failed"));
            }
        }
    };

    // Check for existing consent
    let has_consent = store
        .check_consent(&subject, &client_id, &valid_scopes)
        .unwrap_or(false);

    if has_consent || auth.providers.is_empty() {
        // Auto-approve — generate auth code and redirect
        return issue_auth_code(
            &auth,
            store,
            &client_id,
            &redirect_uri,
            &subject,
            &valid_scopes,
            &code_challenge,
            &state,
        )
        .await;
    }

    // Show consent page
    let user_name = session_data
        .as_ref()
        .and_then(|d| d.user.name.clone())
        .unwrap_or_else(|| subject.clone());

    let scope_descriptions: Vec<(&str, &str)> = valid_scopes
        .iter()
        .map(|s| match s.as_str() {
            "tools:read" => ("tools:read", "List available tools"),
            "tools:execute" => ("tools:execute", "Execute tools on your behalf"),
            other => (other, other),
        })
        .collect();

    let html = crate::authz_server::consent::render_consent_page(
        &client.client_name,
        &client_id,
        &user_name,
        &scope_descriptions,
        &redirect_uri,
        &code_challenge,
        &scope,
        &state,
    );

    Response::builder()
        .status(200)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(axum::body::Body::from(html))
        .unwrap_or_else(|_| error_response(500, "server_error", "Render failed"))
}

/// POST /_rex/auth/authorize — Consent approval/denial.
pub async fn authorize_post_handler(
    State(auth): State<Arc<AuthServer>>,
    headers: axum::http::HeaderMap,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> Response {
    let action = form.get("action").map(|s| s.as_str()).unwrap_or("deny");

    let client_id = match form.get("client_id") {
        Some(id) => id.clone(),
        None => return error_response(400, "invalid_request", "Missing client_id"),
    };

    let redirect_uri = match form.get("redirect_uri") {
        Some(uri) => uri.clone(),
        None => return error_response(400, "invalid_request", "Missing redirect_uri"),
    };

    let code_challenge = match form.get("code_challenge") {
        Some(c) => c.clone(),
        None => return error_response(400, "invalid_request", "Missing code_challenge"),
    };

    let scope = form.get("scope").cloned().unwrap_or_default();
    let state = form.get("state").cloned().unwrap_or_default();

    if action == "deny" {
        let redirect = format!(
            "{redirect_uri}?error=access_denied&error_description=User%20denied%20consent&state={state}"
        );
        return Response::builder()
            .status(302)
            .header("Location", redirect)
            .body(axum::body::Body::empty())
            .unwrap_or_else(|_| error_response(500, "server_error", "Redirect failed"));
    }

    // Get user subject from session
    let cookie_header = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let cookies = parse_cookies(cookie_header);

    let subject = cookies
        .get(&auth.config.session.cookie_name)
        .and_then(|encrypted| session::decrypt_session(encrypted, &auth.session_key))
        .map(|data| data.user.id)
        .unwrap_or_else(|| "owner".to_string());

    let store = match &auth.store {
        Some(s) => s,
        None => return error_response(500, "server_error", "Auth store not initialized"),
    };

    // Save consent decision
    let valid_scopes = crate::scopes::parse_scopes(&scope);

    if let Err(e) = store.store_consent(subject.clone(), client_id.clone(), valid_scopes.clone()) {
        tracing::error!("Failed to save consent: {e}");
    }

    issue_auth_code(
        &auth,
        store,
        &client_id,
        &redirect_uri,
        &subject,
        &valid_scopes,
        &code_challenge,
        &state,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn issue_auth_code(
    _auth: &AuthServer,
    store: &crate::store::FileStore,
    client_id: &str,
    redirect_uri: &str,
    subject: &str,
    scopes: &[String],
    code_challenge: &str,
    state: &str,
) -> Response {
    let code = match store.store_auth_code(
        client_id.to_string(),
        redirect_uri.to_string(),
        subject.to_string(),
        crate::scopes::format_scopes(scopes),
        code_challenge.to_string(),
    ) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to create auth code: {e}");
            return error_response(500, "server_error", "Failed to create authorization code");
        }
    };

    let mut redirect = format!("{redirect_uri}?code={code}");
    if !state.is_empty() {
        redirect.push_str(&format!("&state={state}"));
    }

    Response::builder()
        .status(302)
        .header("Location", redirect)
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| error_response(500, "server_error", "Redirect failed"))
}

/// Validate that a redirect_uri matches one of the registered URIs.
///
/// Supports wildcard port matching: `http://localhost:*/callback` matches
/// `http://localhost:8080/callback`.
fn validate_redirect_uri(uri: &str, registered: &[String]) -> bool {
    for reg in registered {
        if reg == uri {
            return true;
        }
        // Wildcard port matching
        if reg.contains(":*") {
            let pattern = reg.replace(":*", ":");
            if let Some(prefix) = pattern.strip_suffix('/') {
                // Match prefix + port + rest
                if let Ok(parsed) = url::Url::parse(uri) {
                    let no_port =
                        format!("{}://{}:", parsed.scheme(), parsed.host_str().unwrap_or(""));
                    if no_port
                        == prefix
                            .split_at(prefix.rfind(':').unwrap_or(0))
                            .0
                            .to_string()
                            + ":"
                    {
                        return true;
                    }
                }
            }
            // Simple approach: replace :* with :\d+ pattern match
            let parts: Vec<&str> = reg.splitn(2, ":*").collect();
            if parts.len() == 2 {
                if let Some(rest) = uri.strip_prefix(parts[0]) {
                    if let Some(colon_rest) = rest.strip_prefix(':') {
                        // Skip digits
                        let after_port: String = colon_rest
                            .chars()
                            .skip_while(|c| c.is_ascii_digit())
                            .collect();
                        if after_port == parts[1] {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn error_response(status: u16, error: &str, description: &str) -> Response {
    let body = serde_json::json!({
        "error": error,
        "error_description": description,
    });

    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(
            serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
        ))
        .unwrap_or_else(|_| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error",
            )
                .into_response()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_redirect_uri_exact() {
        let registered = vec!["http://localhost:8080/callback".to_string()];
        assert!(validate_redirect_uri(
            "http://localhost:8080/callback",
            &registered
        ));
        assert!(!validate_redirect_uri(
            "http://localhost:9090/callback",
            &registered
        ));
    }

    #[test]
    fn test_validate_redirect_uri_wildcard_port() {
        let registered = vec!["http://localhost:*/callback".to_string()];
        assert!(validate_redirect_uri(
            "http://localhost:8080/callback",
            &registered
        ));
        assert!(validate_redirect_uri(
            "http://localhost:3000/callback",
            &registered
        ));
        assert!(!validate_redirect_uri(
            "http://evil.com:8080/callback",
            &registered
        ));
    }
}
