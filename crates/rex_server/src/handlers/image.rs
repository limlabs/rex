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

    // Reject paths with traversal components before touching the filesystem
    if url_path.split('/').any(|seg| seg == ".." || seg == ".") {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }

    let file_path = state.project_root.join("public").join(url_path);

    // Prevent path traversal — both canonicalizations must succeed
    // and the resolved path must be inside public/
    let public_dir = state.project_root.join("public");
    let public_canonical = match public_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "image not found").into_response(),
    };
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "image not found").into_response(),
    };
    if !canonical.starts_with(&public_canonical) {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::image_handler;
    use crate::handlers::test_support::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use std::path::PathBuf;
    use tower::ServiceExt;

    /// Create a minimal valid JPEG in a unique temp directory and return the path.
    /// Uses a per-test UUID subdirectory to avoid conflicts between parallel tests.
    fn setup_image_dir() -> PathBuf {
        let test_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rex-image-test-{test_id}"));
        let public = dir.join("public").join("images");
        std::fs::create_dir_all(&public).unwrap();

        // Minimal 1x1 JPEG (valid JFIF)
        let jpeg_bytes: Vec<u8> = vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00,
            0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06,
            0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D,
            0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12, 0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D,
            0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28,
            0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32,
            0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01,
            0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01,
            0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02,
            0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10,
            0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00,
            0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06,
            0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42,
            0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16,
            0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37,
            0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55,
            0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73,
            0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
            0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5,
            0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA,
            0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6,
            0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA,
            0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA, 0x00, 0x08,
            0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x7B, 0x94, 0x11, 0x00, 0x00, 0x00, 0xFF, 0xD9,
        ];

        std::fs::write(public.join("hero.jpg"), &jpeg_bytes).unwrap();
        dir
    }

    fn build_image_app(project_root: PathBuf) -> Router {
        TestAppBuilder::new()
            .project_root(project_root)
            .custom_router(|state| {
                Router::new()
                    .route("/_rex/image", get(image_handler))
                    .with_state(state)
            })
            .build()
    }

    #[tokio::test]
    async fn test_image_handler_invalid_width() {
        let dir = setup_image_dir();
        let app = build_image_app(dir.clone());

        let resp = app
            .oneshot(
                Request::get("/_rex/image?url=/images/hero.jpg&w=5&q=75")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_image_handler_unsupported_format() {
        let dir = setup_image_dir();
        let app = build_image_app(dir.clone());

        let resp = app
            .oneshot(
                Request::get("/_rex/image?url=/images/hero.jpg&w=64&q=75&f=bmp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_image_handler_not_found() {
        let dir = setup_image_dir();
        let app = build_image_app(dir.clone());

        let resp = app
            .oneshot(
                Request::get("/_rex/image?url=/images/missing.jpg&w=64&q=75")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_image_handler_path_traversal() {
        let dir = setup_image_dir();
        let app = build_image_app(dir.clone());

        let resp = app
            .oneshot(
                Request::get("/_rex/image?url=/../../../etc/passwd&w=64&q=75")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be rejected (either NOT_FOUND or BAD_REQUEST)
        assert!(
            resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::BAD_REQUEST,
            "path traversal should be blocked, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn test_image_handler_optimizes_jpeg() {
        let dir = setup_image_dir();
        let app = build_image_app(dir.clone());

        let resp = app
            .oneshot(
                Request::get("/_rex/image?url=/images/hero.jpg&w=64&q=75&f=jpeg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("jpeg"));
        assert!(resp
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("immutable"));
    }
}
