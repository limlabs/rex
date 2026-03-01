/// Render the built-in consent page HTML.
///
/// Self-contained HTML (no external dependencies), similar to `dev_error_overlay`.
#[allow(clippy::too_many_arguments)]
pub fn render_consent_page(
    client_name: &str,
    client_id: &str,
    user_name: &str,
    scopes: &[(&str, &str)],
    redirect_uri: &str,
    code_challenge: &str,
    scope: &str,
    state: &str,
    csrf_token: &str,
) -> String {
    let escaped_client = html_escape(client_name);
    let escaped_user = html_escape(user_name);
    let escaped_client_id = html_escape(client_id);
    let escaped_redirect_uri = html_escape(redirect_uri);
    let escaped_code_challenge = html_escape(code_challenge);
    let escaped_scope = html_escape(scope);
    let escaped_state = html_escape(state);
    let escaped_csrf_token = html_escape(csrf_token);

    let scope_items: String = scopes
        .iter()
        .map(|(id, desc)| {
            format!(
                r#"<li><code>{}</code> — {}</li>"#,
                html_escape(id),
                html_escape(desc)
            )
        })
        .collect();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Authorize {escaped_client}</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: #f5f5f5; display: flex; justify-content: center; align-items: center; min-height: 100vh; padding: 20px; }}
.card {{ background: white; border-radius: 12px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); max-width: 420px; width: 100%; padding: 32px; }}
h1 {{ font-size: 20px; margin-bottom: 8px; color: #1a1a1a; }}
.subtitle {{ color: #666; font-size: 14px; margin-bottom: 24px; }}
.client-id {{ font-size: 12px; color: #999; font-family: monospace; }}
h2 {{ font-size: 14px; color: #333; margin-bottom: 12px; text-transform: uppercase; letter-spacing: 0.5px; }}
ul {{ list-style: none; margin-bottom: 24px; }}
li {{ padding: 8px 12px; background: #f9f9f9; border-radius: 6px; margin-bottom: 6px; font-size: 14px; }}
li code {{ background: #e8e8e8; padding: 2px 6px; border-radius: 3px; font-size: 12px; }}
.buttons {{ display: flex; gap: 12px; }}
button {{ flex: 1; padding: 12px; border: none; border-radius: 8px; font-size: 14px; font-weight: 600; cursor: pointer; transition: opacity 0.2s; }}
button:hover {{ opacity: 0.9; }}
.approve {{ background: #2563eb; color: white; }}
.deny {{ background: #e5e7eb; color: #374151; }}
</style>
</head>
<body>
<div class="card">
<h1>{escaped_client}</h1>
<p class="subtitle">wants to access your account as <strong>{escaped_user}</strong></p>
<p class="client-id">Client ID: {escaped_client_id}</p>

<h2 style="margin-top: 20px;">Permissions requested</h2>
<ul>{scope_items}</ul>

<div class="buttons">
<form method="POST" action="/_rex/auth/authorize" style="flex:1;display:flex">
<input type="hidden" name="action" value="approve">
<input type="hidden" name="client_id" value="{escaped_client_id}">
<input type="hidden" name="redirect_uri" value="{escaped_redirect_uri}">
<input type="hidden" name="code_challenge" value="{escaped_code_challenge}">
<input type="hidden" name="scope" value="{escaped_scope}">
<input type="hidden" name="state" value="{escaped_state}">
<input type="hidden" name="csrf_token" value="{escaped_csrf_token}">
<button type="submit" class="approve" style="flex:1">Approve</button>
</form>
<form method="POST" action="/_rex/auth/authorize" style="flex:1;display:flex">
<input type="hidden" name="action" value="deny">
<input type="hidden" name="client_id" value="{escaped_client_id}">
<input type="hidden" name="redirect_uri" value="{escaped_redirect_uri}">
<input type="hidden" name="code_challenge" value="{escaped_code_challenge}">
<input type="hidden" name="scope" value="{escaped_scope}">
<input type="hidden" name="state" value="{escaped_state}">
<input type="hidden" name="csrf_token" value="{escaped_csrf_token}">
<button type="submit" class="deny" style="flex:1">Deny</button>
</form>
</div>
</div>
</body>
</html>"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
