use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use rex_core::ErrorBuffer;
use rex_core::PageType;
use serde::Serialize;
use std::sync::Arc;

use crate::state::{snapshot, AppState};

#[derive(Serialize)]
pub struct StatusResponse {
    pub build_id: String,
    pub project_root: String,
    pub route_count: usize,
    pub api_route_count: usize,
    pub app_route_count: usize,
    pub has_middleware: bool,
    pub has_mcp_tools: bool,
}

#[derive(Serialize)]
pub struct RoutesResponse {
    pub pages: Vec<RouteInfo>,
    pub api: Vec<RouteInfo>,
    pub app: Vec<RouteInfo>,
}

#[derive(Serialize)]
pub struct RouteInfo {
    pub pattern: String,
    pub file_path: String,
    pub page_type: String,
    pub dynamic_segments: Vec<String>,
}

pub async fn status_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let hot = snapshot(&state);
    let app_route_count = hot
        .app_route_trie
        .as_ref()
        .map(|t| t.routes().len())
        .unwrap_or(0);

    Json(StatusResponse {
        build_id: hot.build_id.clone(),
        project_root: state.project_root.to_string_lossy().to_string(),
        route_count: hot.route_trie.routes().len(),
        api_route_count: hot.api_route_trie.routes().len(),
        app_route_count,
        has_middleware: hot.has_middleware,
        has_mcp_tools: hot.has_mcp_tools,
    })
}

fn page_type_str(pt: &PageType) -> &'static str {
    match pt {
        PageType::Regular => "Regular",
        PageType::Api => "Api",
        PageType::AppApi => "AppApi",
        PageType::App => "App",
        PageType::Document => "Document",
        PageType::Error => "Error",
        PageType::NotFound => "NotFound",
    }
}

fn segment_name(seg: &rex_core::DynamicSegment) -> String {
    match seg {
        rex_core::DynamicSegment::Single(name) => name.clone(),
        rex_core::DynamicSegment::CatchAll(name) => format!("...{name}"),
        rex_core::DynamicSegment::OptionalCatchAll(name) => format!("[[...{name}]]"),
    }
}

fn route_to_info(route: &rex_core::Route) -> RouteInfo {
    RouteInfo {
        pattern: route.pattern.clone(),
        file_path: route.file_path.to_string_lossy().to_string(),
        page_type: page_type_str(&route.page_type).to_string(),
        dynamic_segments: route.dynamic_segments.iter().map(segment_name).collect(),
    }
}

pub async fn routes_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let hot = snapshot(&state);

    let pages: Vec<RouteInfo> = hot
        .route_trie
        .routes()
        .into_iter()
        .map(route_to_info)
        .collect();
    let api: Vec<RouteInfo> = hot
        .api_route_trie
        .routes()
        .into_iter()
        .map(route_to_info)
        .collect();
    let app: Vec<RouteInfo> = hot
        .app_route_trie
        .as_ref()
        .map(|t| t.routes().into_iter().map(route_to_info).collect())
        .unwrap_or_default();

    Json(RoutesResponse { pages, api, app })
}

pub async fn errors_handler(Extension(buffer): Extension<ErrorBuffer>) -> impl IntoResponse {
    Json(buffer.snapshot())
}
