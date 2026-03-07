use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use tracing::{debug, error};

use super::AppState;

/// Query parameters for the image optimization endpoint.
#[derive(serde::Deserialize)]
pub struct ImageQuery {
    pub url: String,
    pub w: u32,
    #[serde(default = "default_quality")]
    pub q: u8,
    pub f: Option<String>,
}

fn default_quality() -> u8 {
    75
}

/// Image optimization endpoint: GET /_rex/image?url=/images/hero.jpg&w=640&q=75&f=webp
pub async fn image_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ImageQuery>,
) -> Response {
    // Validate width
    if query.w < 16 || query.w > 4096 {
        return (StatusCode::BAD_REQUEST, "width must be 16\u{2013}4096").into_response();
    }

    // Determine output format: explicit `f=` param takes priority,
    // otherwise preserve PNG for .png sources (keeps transparency),
    // and use JPEG for everything else.
    let explicit_format = match &query.f {
        Some(f) => match f.as_str() {
            "webp" => Some(rex_image::OutputFormat::WebP),
            "jpeg" | "jpg" => Some(rex_image::OutputFormat::Jpeg),
            "png" => Some(rex_image::OutputFormat::Png),
            _ => return (StatusCode::BAD_REQUEST, "unsupported format").into_response(),
        },
        None => None,
    };
    let format = explicit_format.unwrap_or_else(|| {
        if query.url.ends_with(".png") {
            rex_image::OutputFormat::Png
        } else {
            let accept = headers
                .get("accept")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            rex_image::negotiate_format(accept)
        }
    });

    // Check cache
    let cache_key =
        rex_image::ImageCache::cache_key(&query.url, query.w, query.q, format.extension());

    if let Some(data) = state.image_cache.get(&cache_key) {
        return Response::builder()
            .header("Content-Type", format.content_type())
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(Body::from(data))
            .expect("response build");
    }

    // Resolve source file from public/ directory (only local files)
    let url_path = query.url.trim_start_matches('/');
    let file_path = state.project_root.join("public").join(url_path);

    // Prevent path traversal
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "image not found").into_response(),
    };
    let public_dir = state.project_root.join("public");
    if let Ok(public_canonical) = public_dir.canonicalize() {
        if !canonical.starts_with(&public_canonical) {
            return (StatusCode::BAD_REQUEST, "invalid path").into_response();
        }
    }

    let src_bytes = match std::fs::read(&canonical) {
        Ok(data) => data,
        Err(_) => return (StatusCode::NOT_FOUND, "image not found").into_response(),
    };

    let params = rex_image::OptimizeParams {
        width: query.w,
        quality: query.q,
        format,
    };

    match rex_image::optimize(&src_bytes, &params) {
        Ok(optimized) => {
            // Cache the result (ignore cache write errors)
            if let Err(e) = state.image_cache.put(&cache_key, &optimized) {
                debug!("image cache write failed: {e}");
            }

            Response::builder()
                .header("Content-Type", format.content_type())
                .header("Cache-Control", "public, max-age=31536000, immutable")
                .body(Body::from(optimized))
                .expect("response build")
        }
        Err(e) => {
            error!("image optimization failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
