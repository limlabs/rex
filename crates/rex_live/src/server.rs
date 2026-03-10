//! Live mode server — routes requests across multiple mounted projects.

use crate::handler;
use crate::project::{LiveProject, LiveProjectConfig};
use anyhow::Result;
use axum::routing::{any, get};
use axum::Router;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tracing::info;

/// Configuration for a single mount point.
pub struct MountConfig {
    /// URL path prefix (e.g., "/dashboard")
    pub prefix: String,
    /// Source directory path
    pub source: std::path::PathBuf,
}

/// Configuration for the live server.
pub struct LiveServerConfig {
    pub mounts: Vec<MountConfig>,
    pub port: u16,
    pub host: IpAddr,
    pub workers_per_project: usize,
}

/// The live mode server, holding all mounted projects.
pub struct LiveServer {
    /// Projects sorted by prefix length (longest first) for matching
    projects: Vec<Arc<LiveProject>>,
}

impl LiveServer {
    /// Create a new live server from config.
    pub fn new(config: &LiveServerConfig) -> Result<Arc<Self>> {
        let mut projects = Vec::new();

        for mount in &config.mounts {
            let prefix = normalize_prefix(&mount.prefix);
            info!(
                prefix = %prefix,
                source = %mount.source.display(),
                "Mounting live project"
            );

            let project = LiveProject::new(LiveProjectConfig {
                prefix: prefix.clone(),
                source_path: mount.source.clone(),
                workers: config.workers_per_project,
            })?;

            projects.push(project);
        }

        // Sort by prefix length descending (longest prefix match first)
        projects.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));

        Ok(Arc::new(Self { projects }))
    }

    /// Match a request path to a project by longest prefix.
    /// Returns the project and the remaining path after stripping the prefix.
    pub fn match_project<'a>(&self, path: &'a str) -> Option<(Arc<LiveProject>, &'a str)> {
        for project in &self.projects {
            if project.prefix == "/" {
                return Some((project.clone(), path));
            }
            if path == project.prefix || path.starts_with(&format!("{}/", project.prefix)) {
                let remaining = &path[project.prefix.len()..];
                let remaining = if remaining.is_empty() { "/" } else { remaining };
                return Some((project.clone(), remaining));
            }
        }
        None
    }

    /// Build the Axum router for serving live mode.
    pub fn build_router(self: &Arc<Self>) -> Router {
        // For each project, serve its static assets under /_rex/static
        // using the project's .rex/build/client/ directory
        let mut router = Router::new();

        // Status endpoint
        router = router.route("/_rex/live/status", get(status_handler));

        // For each project, mount its static dir
        for project in &self.projects {
            let static_dir = project.static_dir();
            let prefix = if project.prefix == "/" {
                String::new()
            } else {
                project.prefix.clone()
            };

            // Nest the static file service at the project's prefix
            let static_path = format!("{prefix}/_rex/static");
            if static_dir.exists() {
                router = router.nest_service(&static_path, ServeDir::new(&static_dir));
            }
        }

        // Fallback: live handler for all other routes
        router
            .fallback(any(handler::live_handler))
            .with_state(Arc::clone(self))
            .layer(CompressionLayer::new().gzip(true))
    }

    /// Start the live server.
    pub async fn serve(self: Arc<Self>, port: u16, host: IpAddr) -> Result<()> {
        let router = self.build_router();
        let addr = SocketAddr::new(host, port);

        info!("Rex live server listening on http://{addr}");

        for project in &self.projects {
            info!("  {} → {}", project.prefix, project.source_root().display());
        }

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, router).await?;

        Ok(())
    }

    /// Get all mounted projects (for status/metrics).
    pub fn projects(&self) -> &[Arc<LiveProject>] {
        &self.projects
    }
}

/// Normalize a mount prefix: ensure leading slash, no trailing slash.
fn normalize_prefix(prefix: &str) -> String {
    let mut p = prefix.to_string();
    if !p.starts_with('/') {
        p.insert(0, '/');
    }
    if p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    p
}

/// Simple status endpoint for health checks.
async fn status_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "mode": "live"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_leading_slash() {
        assert_eq!(normalize_prefix("admin"), "/admin");
    }

    #[test]
    fn normalize_removes_trailing_slash() {
        assert_eq!(normalize_prefix("/admin/"), "/admin");
    }

    #[test]
    fn normalize_preserves_root() {
        assert_eq!(normalize_prefix("/"), "/");
    }

    #[test]
    fn normalize_already_correct() {
        assert_eq!(normalize_prefix("/dashboard"), "/dashboard");
    }

    #[test]
    fn normalize_nested_prefix() {
        assert_eq!(normalize_prefix("/admin/settings/"), "/admin/settings");
    }

    #[test]
    fn normalize_bare_slash() {
        assert_eq!(normalize_prefix(""), "/");
    }
}
