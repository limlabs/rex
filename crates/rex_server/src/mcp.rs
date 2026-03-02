use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::debug;

use crate::handlers::AppState;

// --- JSON-RPC 2.0 types ---

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
    id: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

// --- MCP-specific types ---

#[derive(Debug, Serialize)]
struct McpServerInfo {
    name: String,
    version: String,
}

#[derive(Debug, Serialize)]
struct McpCapabilities {
    tools: McpToolsCapability,
}

#[derive(Debug, Serialize)]
struct McpToolsCapability {}

#[derive(Debug, Serialize)]
struct McpInitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    capabilities: McpCapabilities,
    #[serde(rename = "serverInfo")]
    server_info: McpServerInfo,
}

#[derive(Debug, Serialize)]
struct McpToolInfo {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(rename = "inputSchema")]
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct McpToolsListResult {
    tools: Vec<McpToolInfo>,
}

#[derive(Debug, Serialize)]
struct McpToolCallResult {
    content: Vec<McpContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

#[derive(Debug, Serialize)]
struct McpContent {
    #[serde(rename = "type")]
    content_type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct McpToolCallParams {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

// --- Handler ---

fn json_rpc_response(id: serde_json::Value, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        result: Some(result),
        error: None,
        id,
    }
}

fn json_rpc_error(id: serde_json::Value, code: i32, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        result: None,
        error: Some(JsonRpcError {
            code,
            message,
            data: None,
        }),
        id,
    }
}

/// Serialize a JSON-RPC response to a string, falling back to an empty object on error.
fn serialize_response(resp: &JsonRpcResponse) -> String {
    serde_json::to_string(resp).unwrap_or_default()
}

/// Convert a serializable value to `serde_json::Value`.
/// This only fails for types that can't be represented as JSON (e.g., maps with non-string keys),
/// which none of our MCP types have.
fn to_json_value<T: Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value).expect("MCP types are always JSON-serializable")
}

pub async fn mcp_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Check if MCP tools are available
    let hot = match state.hot.read() {
        Ok(guard) => Arc::clone(&guard),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "State lock poisoned").into_response()
        }
    };

    if !hot.has_mcp_tools {
        return (StatusCode::NOT_FOUND, "No MCP tools configured").into_response();
    }

    // Auth gate: if auth is configured with MCP enabled, require a valid Bearer token
    if let Some(ref auth) = state.auth {
        if auth.config.mcp.enabled {
            let token = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "));

            match token {
                None => {
                    return (
                        StatusCode::UNAUTHORIZED,
                        [("www-authenticate", "Bearer")],
                        "Missing or invalid Authorization header",
                    )
                        .into_response();
                }
                Some(token) => {
                    let key_manager = auth
                        .key_manager
                        .as_ref()
                        .expect("key_manager is always Some when mcp.enabled");
                    let decoding_keys = match key_manager.decoding_keys() {
                        Ok(keys) => keys,
                        Err(_) => {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Failed to load auth keys",
                            )
                                .into_response();
                        }
                    };
                    if let Err(_e) =
                        rex_auth::jwt::validate_access_token(token, &decoding_keys, auth.issuer())
                    {
                        return (
                            StatusCode::UNAUTHORIZED,
                            [("www-authenticate", "Bearer error=\"invalid_token\"")],
                            "Invalid or expired token",
                        )
                            .into_response();
                    }
                }
            }
        }
    }

    // Parse JSON-RPC request
    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(e) => {
            let resp = json_rpc_error(serde_json::Value::Null, -32700, format!("Parse error: {e}"));
            return (
                StatusCode::OK,
                [("content-type", "application/json")],
                serialize_response(&resp),
            )
                .into_response();
        }
    };

    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        let resp = json_rpc_error(
            request.id.unwrap_or(serde_json::Value::Null),
            -32600,
            "Invalid Request: jsonrpc must be \"2.0\"".to_string(),
        );
        return (
            StatusCode::OK,
            [("content-type", "application/json")],
            serialize_response(&resp),
        )
            .into_response();
    }

    // Handle notifications (no id = notification, return 204)
    if request.id.is_none() {
        debug!(method = %request.method, "MCP notification");
        return StatusCode::NO_CONTENT.into_response();
    }

    let id = request.id.unwrap_or(serde_json::Value::Null);

    debug!(method = %request.method, "MCP request");

    let response = match request.method.as_str() {
        "initialize" => {
            let result = McpInitializeResult {
                protocol_version: "2025-03-26".to_string(),
                capabilities: McpCapabilities {
                    tools: McpToolsCapability {},
                },
                server_info: McpServerInfo {
                    name: "rex".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
            };
            json_rpc_response(id, to_json_value(result))
        }

        "ping" => json_rpc_response(id, serde_json::json!({})),

        "tools/list" => {
            let tools_json = state.isolate_pool.execute(|iso| iso.list_mcp_tools()).await;

            match tools_json {
                Ok(Ok(Some(json_str))) => {
                    let tools_raw: Vec<serde_json::Value> =
                        serde_json::from_str(&json_str).unwrap_or_default();
                    let tools: Vec<McpToolInfo> = tools_raw
                        .into_iter()
                        .map(|t| McpToolInfo {
                            name: t["name"].as_str().unwrap_or("").to_string(),
                            description: t["description"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string()),
                            input_schema: t["parameters"].clone(),
                        })
                        .collect();
                    let result = McpToolsListResult { tools };
                    json_rpc_response(id, to_json_value(result))
                }
                Ok(Ok(None)) | Ok(Err(_)) => {
                    let result = McpToolsListResult { tools: vec![] };
                    json_rpc_response(id, to_json_value(result))
                }
                Err(e) => json_rpc_error(id, -32603, format!("Internal error: {e}")),
            }
        }

        "tools/call" => {
            let call_params: McpToolCallParams = match serde_json::from_value(request.params) {
                Ok(p) => p,
                Err(e) => {
                    return (
                        StatusCode::OK,
                        [("content-type", "application/json")],
                        serialize_response(&json_rpc_error(
                            id,
                            -32602,
                            format!("Invalid params: {e}"),
                        )),
                    )
                        .into_response();
                }
            };

            let tool_name = call_params.name;
            let args_json = serde_json::to_string(&call_params.arguments).unwrap_or_default();

            let result = state
                .isolate_pool
                .execute(move |iso| iso.call_mcp_tool(&tool_name, &args_json))
                .await;

            match result {
                Ok(Ok(result_json)) => {
                    let call_result = McpToolCallResult {
                        content: vec![McpContent {
                            content_type: "text".to_string(),
                            text: result_json,
                        }],
                        is_error: None,
                    };
                    json_rpc_response(id, to_json_value(call_result))
                }
                Ok(Err(e)) | Err(e) => {
                    let call_result = McpToolCallResult {
                        content: vec![McpContent {
                            content_type: "text".to_string(),
                            text: format!("Tool execution error: {e}"),
                        }],
                        is_error: Some(true),
                    };
                    json_rpc_response(id, to_json_value(call_result))
                }
            }
        }

        _ => json_rpc_error(id, -32601, format!("Method not found: {}", request.method)),
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        serialize_response(&response),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_rpc_request() {
        let json = r#"{"jsonrpc":"2.0","method":"initialize","params":{},"id":1}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).expect("valid JSON-RPC request");
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn test_parse_json_rpc_notification() {
        let json = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).expect("valid JSON-RPC notification");
        assert_eq!(req.method, "notifications/initialized");
        assert!(req.id.is_none());
    }

    #[test]
    fn test_json_rpc_response_serialization() {
        let resp = json_rpc_response(serde_json::json!(1), serde_json::json!({"result": "ok"}));
        let json = serde_json::to_string(&resp).expect("serializable response");
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_json_rpc_error_serialization() {
        let resp = json_rpc_error(serde_json::json!(2), -32601, "Method not found".to_string());
        let json = serde_json::to_string(&resp).expect("serializable error");
        assert!(json.contains("\"error\""));
        assert!(json.contains("-32601"));
        assert!(!json.contains("\"result\""));
    }

    #[test]
    fn test_parse_tool_call_params() {
        let json = r#"{"name":"search","arguments":{"query":"test"}}"#;
        let params: McpToolCallParams = serde_json::from_str(json).expect("valid tool call params");
        assert_eq!(params.name, "search");
        assert_eq!(params.arguments["query"], "test");
    }

    #[test]
    fn test_mcp_initialize_result() {
        let result = McpInitializeResult {
            protocol_version: "2025-03-26".to_string(),
            capabilities: McpCapabilities {
                tools: McpToolsCapability {},
            },
            server_info: McpServerInfo {
                name: "rex".to_string(),
                version: "0.1.0".to_string(),
            },
        };
        let json = serde_json::to_value(&result).expect("serializable init result");
        assert_eq!(json["protocolVersion"], "2025-03-26");
        assert_eq!(json["serverInfo"]["name"], "rex");
        assert!(json["capabilities"]["tools"].is_object());
    }
}
