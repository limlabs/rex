use anyhow::Result;
use rex_core::instance::{self, InstanceInfo};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

// --- JSON-RPC 2.0 types (local, decoupled from rex_server::mcp) ---

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
    id: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: serde_json::Value,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// --- MCP types ---

#[derive(Serialize)]
struct McpInitResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: &'static str,
    capabilities: McpCapabilities,
    #[serde(rename = "serverInfo")]
    server_info: McpServerInfo,
}

#[derive(Serialize)]
struct McpCapabilities {
    tools: serde_json::Value,
}

#[derive(Serialize)]
struct McpServerInfo {
    name: &'static str,
    version: &'static str,
}

#[derive(Serialize)]
struct McpToolInfo {
    name: &'static str,
    description: &'static str,
    #[serde(rename = "inputSchema")]
    input_schema: serde_json::Value,
}

#[derive(Serialize)]
struct McpToolResult {
    content: Vec<McpContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

#[derive(Serialize)]
struct McpContent {
    #[serde(rename = "type")]
    content_type: &'static str,
    text: String,
}

#[derive(Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

// --- Helpers ---

fn ok_response(id: serde_json::Value, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        result: Some(result),
        error: None,
        id,
    }
}

fn err_response(id: serde_json::Value, code: i32, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        result: None,
        error: Some(JsonRpcError { code, message }),
        id,
    }
}

fn tool_text(text: String) -> serde_json::Value {
    serde_json::to_value(McpToolResult {
        content: vec![McpContent {
            content_type: "text",
            text,
        }],
        is_error: None,
    })
    .expect("McpToolResult is always serializable")
}

fn tool_error(text: String) -> serde_json::Value {
    serde_json::to_value(McpToolResult {
        content: vec![McpContent {
            content_type: "text",
            text,
        }],
        is_error: Some(true),
    })
    .expect("McpToolResult is always serializable")
}

/// Resolve which instance to query. If only one is running, use it implicitly.
/// Otherwise, requires `port` or `pid` in the arguments.
fn resolve_instance(args: &serde_json::Value) -> std::result::Result<InstanceInfo, String> {
    let instances = instance::list_instances();

    if let Some(port) = args.get("port").and_then(|v| v.as_u64()) {
        return instances
            .into_iter()
            .find(|i| i.port == port as u16)
            .ok_or_else(|| format!("No Rex instance running on port {port}"));
    }

    if let Some(pid) = args.get("pid").and_then(|v| v.as_u64()) {
        return instances
            .into_iter()
            .find(|i| i.pid == pid as u32)
            .ok_or_else(|| format!("No Rex instance with PID {pid}"));
    }

    match instances.len() {
        0 => Err("No Rex dev instances running".to_string()),
        1 => Ok(instances.into_iter().next().expect("checked len == 1")),
        n => Err(format!(
            "{n} Rex instances running — specify 'port' or 'pid' to disambiguate"
        )),
    }
}

fn instance_select_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "port": { "type": "integer", "description": "Port of the Rex dev instance" },
            "pid": { "type": "integer", "description": "PID of the Rex dev instance" }
        }
    })
}

fn tools_list() -> Vec<McpToolInfo> {
    vec![
        McpToolInfo {
            name: "rex_list_instances",
            description: "List all running Rex dev server instances",
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
        },
        McpToolInfo {
            name: "rex_get_status",
            description: "Get status of a Rex dev server (build ID, route counts, features)",
            input_schema: instance_select_schema(),
        },
        McpToolInfo {
            name: "rex_get_routes",
            description: "List all routes (pages, API, app) from a Rex dev server",
            input_schema: instance_select_schema(),
        },
        McpToolInfo {
            name: "rex_get_errors",
            description: "Get recent build/TypeScript errors from a Rex dev server",
            input_schema: instance_select_schema(),
        },
    ]
}

async fn http_get(url: &str) -> std::result::Result<String, String> {
    let resp = reqwest::get(url)
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {e}"))?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {body}"));
    }
    Ok(body)
}

async fn handle_tool_call(name: &str, args: &serde_json::Value) -> serde_json::Value {
    match name {
        "rex_list_instances" => {
            let instances = instance::list_instances();
            let json = serde_json::to_string_pretty(&instances).unwrap_or_default();
            tool_text(json)
        }
        "rex_get_status" | "rex_get_routes" | "rex_get_errors" => {
            let inst = match resolve_instance(args) {
                Ok(i) => i,
                Err(e) => return tool_error(e),
            };
            let endpoint = match name {
                "rex_get_status" => "status",
                "rex_get_routes" => "routes",
                "rex_get_errors" => "errors",
                _ => unreachable!(),
            };
            let url = format!("http://{}:{}/_rex/dev/{endpoint}", inst.host, inst.port);
            match http_get(&url).await {
                Ok(body) => tool_text(body),
                Err(e) => tool_error(e),
            }
        }
        _ => tool_error(format!("Unknown tool: {name}")),
    }
}

/// Process a JSON-RPC request. Returns `None` for notifications (no id).
async fn handle_request(req: JsonRpcRequest) -> Option<JsonRpcResponse> {
    let id = req.id?;

    let resp = match req.method.as_str() {
        "initialize" => {
            let result = McpInitResult {
                protocol_version: "2025-03-26",
                capabilities: McpCapabilities {
                    tools: serde_json::json!({}),
                },
                server_info: McpServerInfo {
                    name: "rex-dev",
                    version: env!("CARGO_PKG_VERSION"),
                },
            };
            ok_response(
                id,
                serde_json::to_value(result).expect("always serializable"),
            )
        }
        "ping" => ok_response(id, serde_json::json!({})),
        "tools/list" => {
            let tools = tools_list();
            ok_response(
                id,
                serde_json::json!({ "tools": serde_json::to_value(&tools).expect("always serializable") }),
            )
        }
        "tools/call" => {
            let params: ToolCallParams = match serde_json::from_value(req.params) {
                Ok(p) => p,
                Err(e) => {
                    return Some(err_response(id, -32602, format!("Invalid params: {e}")));
                }
            };
            let result = handle_tool_call(&params.name, &params.arguments).await;
            ok_response(id, result)
        }
        other => err_response(id, -32601, format!("Method not found: {other}")),
    };

    Some(resp)
}

pub async fn cmd_mcp() -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                let resp =
                    err_response(serde_json::Value::Null, -32700, format!("Parse error: {e}"));
                let mut out = stdout.lock();
                serde_json::to_writer(&mut out, &resp)?;
                writeln!(out)?;
                out.flush()?;
                continue;
            }
        };

        if let Some(resp) = handle_request(req).await {
            let mut out = stdout.lock();
            serde_json::to_writer(&mut out, &resp)?;
            writeln!(out)?;
            out.flush()?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn make_request(method: &str, id: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: serde_json::json!({}),
            id,
        }
    }

    #[tokio::test]
    async fn test_initialize() {
        let req = make_request("initialize", Some(serde_json::json!(1)));
        let resp = handle_request(req).await.unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], "2025-03-26");
        assert_eq!(result["serverInfo"]["name"], "rex-dev");
    }

    #[tokio::test]
    async fn test_ping() {
        let req = make_request("ping", Some(serde_json::json!(2)));
        let resp = handle_request(req).await.unwrap();
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn test_tools_list() {
        let req = make_request("tools/list", Some(serde_json::json!(3)));
        let resp = handle_request(req).await.unwrap();
        let tools = resp.result.unwrap()["tools"].as_array().unwrap().clone();
        assert_eq!(tools.len(), 4);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"rex_list_instances"));
        assert!(names.contains(&"rex_get_errors"));
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let req = make_request("nonexistent", Some(serde_json::json!(4)));
        let resp = handle_request(req).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn test_notification_returns_none() {
        let req = make_request("ping", None);
        assert!(handle_request(req).await.is_none());
    }

    #[tokio::test]
    async fn test_tools_call_list_instances() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!({"name": "rex_list_instances", "arguments": {}}),
            id: Some(serde_json::json!(5)),
        };
        let resp = handle_request(req).await.unwrap();
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn test_tools_call_unknown_tool() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!({"name": "nonexistent_tool", "arguments": {}}),
            id: Some(serde_json::json!(6)),
        };
        let resp = handle_request(req).await.unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
    }

    #[tokio::test]
    async fn test_tools_call_invalid_params() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "tools/call".to_string(),
            params: serde_json::json!("not an object"),
            id: Some(serde_json::json!(7)),
        };
        let resp = handle_request(req).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn test_ok_response_serialization() {
        let resp = ok_response(serde_json::json!(1), serde_json::json!({"ok": true}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_err_response_serialization() {
        let resp = err_response(serde_json::json!(2), -32601, "Not found".into());
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));
        assert!(json.contains("-32601"));
        assert!(!json.contains("\"result\""));
    }

    #[test]
    fn test_tool_text_serialization() {
        let val = tool_text("hello".to_string());
        assert_eq!(val["content"][0]["type"], "text");
        assert_eq!(val["content"][0]["text"], "hello");
        assert!(val.get("isError").is_none());
    }

    #[test]
    fn test_tool_error_serialization() {
        let val = tool_error("oops".to_string());
        assert_eq!(val["isError"], true);
        assert_eq!(val["content"][0]["text"], "oops");
    }

    #[test]
    fn test_tools_list_count() {
        let tools = tools_list();
        assert_eq!(tools.len(), 4);
    }
}
