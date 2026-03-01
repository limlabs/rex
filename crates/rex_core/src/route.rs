use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DataStrategy {
    #[default]
    None,
    GetServerSideProps,
    GetStaticProps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DynamicSegment {
    /// `[slug]` - matches a single path segment
    Single(String),
    /// `[...slug]` - matches one or more path segments
    CatchAll(String),
    /// `[[...slug]]` - matches zero or more path segments
    OptionalCatchAll(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageType {
    Regular,
    Api,      // pages/api/*
    App,      // _app
    Document, // _document
    Error,    // _error
    NotFound, // 404
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    /// The URL pattern, e.g. "/blog/:slug"
    pub pattern: String,
    /// Path to the source file relative to pages/
    pub file_path: PathBuf,
    /// Absolute path to the source file
    pub abs_path: PathBuf,
    /// Dynamic segments extracted from the pattern
    pub dynamic_segments: Vec<DynamicSegment>,
    /// Page type classification
    pub page_type: PageType,
    /// Higher = more specific, used for route priority
    pub specificity: u32,
}

impl Route {
    /// Get the route's module name for JS registry (e.g., "/blog/[slug]" -> "blog/[slug]")
    pub fn module_name(&self) -> String {
        self.file_path
            .with_extension("")
            .to_string_lossy()
            .replace('\\', "/")
    }
}

/// The result of matching a URL against the route trie
#[derive(Debug, Clone)]
pub struct RouteMatch {
    pub route: Route,
    pub params: HashMap<String, String>,
}

/// Context passed to getServerSideProps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSidePropsContext {
    pub params: HashMap<String, String>,
    pub query: HashMap<String, String>,
    #[serde(rename = "resolvedUrl")]
    pub resolved_url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub cookies: HashMap<String, String>,
}

/// Result from getServerSideProps
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ServerSidePropsResult {
    Props {
        props: serde_json::Value,
    },
    Redirect {
        redirect: RedirectConfig,
    },
    NotFound {
        #[serde(rename = "notFound")]
        not_found: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedirectConfig {
    pub destination: String,
    #[serde(default = "default_redirect_status")]
    pub status_code: u16,
    #[serde(default)]
    pub permanent: bool,
}

fn default_redirect_status() -> u16 {
    307
}

// --- MCP tool types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolRoute {
    /// Tool name derived from filename stem (e.g., "search" from "search.ts")
    pub name: String,
    /// Absolute path to the source file
    pub abs_path: PathBuf,
    /// Path relative to the mcp/ directory
    pub file_path: PathBuf,
}

// --- Middleware types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MiddlewareAction {
    Next,
    Redirect,
    Rewrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiddlewareResult {
    pub action: MiddlewareAction,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default = "default_redirect_status")]
    pub status: u16,
    #[serde(default)]
    pub request_headers: HashMap<String, String>,
    #[serde(default)]
    pub response_headers: HashMap<String, String>,
}

// --- Middleware config types (rex.config.json) ---

/// A redirect rule from rex.config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedirectRule {
    /// Source path pattern (supports :param for dynamic segments)
    pub source: String,
    /// Destination path (supports :param references)
    pub destination: String,
    /// HTTP status code (301 or 308 for permanent, 302 or 307 for temporary)
    #[serde(default = "default_redirect_rule_status")]
    pub status_code: u16,
    /// Whether this redirect is permanent (overrides status_code)
    #[serde(default)]
    pub permanent: bool,
}

fn default_redirect_rule_status() -> u16 {
    307
}

/// A rewrite rule from rex.config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteRule {
    /// Source path pattern (supports :param for dynamic segments)
    pub source: String,
    /// Destination path (supports :param references)
    pub destination: String,
}

/// A custom header rule from rex.config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderRule {
    /// Path pattern to match (supports :param for dynamic segments)
    pub source: String,
    /// Headers to add to matching responses
    pub headers: Vec<HeaderEntry>,
}

/// A single header key-value pair
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
}

/// Build-time configuration from rex.config.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Additional module aliases (e.g. `"@components": "./src/components"`)
    #[serde(default)]
    pub alias: HashMap<String, String>,
}

impl BuildConfig {
    /// Resolve alias values that are relative paths against the project root.
    pub fn resolved_aliases(&self, project_root: &Path) -> Vec<(String, Vec<Option<String>>)> {
        self.alias
            .iter()
            .map(|(key, value)| {
                let resolved = if value.starts_with("./") || value.starts_with("../") {
                    project_root.join(value).to_string_lossy().to_string()
                } else {
                    value.clone()
                };
                (key.clone(), vec![Some(resolved)])
            })
            .collect()
    }
}

/// Dev server configuration from rex.config.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DevConfig {
    #[serde(default)]
    pub no_tui: bool,
}

/// Top-level project configuration from rex.config.json
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub redirects: Vec<RedirectRule>,
    #[serde(default)]
    pub rewrites: Vec<RewriteRule>,
    #[serde(default)]
    pub headers: Vec<HeaderRule>,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub dev: DevConfig,
}

impl ProjectConfig {
    /// Load from rex.config.json in the project root. Returns default if file doesn't exist.
    pub fn load(project_root: &std::path::Path) -> Result<Self, crate::RexError> {
        let config_path = project_root.join("rex.config.json");
        if !config_path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| crate::RexError::Config(format!("Failed to read rex.config.json: {e}")))?;
        serde_json::from_str(&content)
            .map_err(|e| crate::RexError::Config(format!("Invalid rex.config.json: {e}")))
    }

    /// Match a request path against a source pattern and return captured params.
    /// Patterns support `:param` for single segments and `*` for catch-all.
    pub fn match_pattern(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
        let pat_segs: Vec<&str> = pattern.trim_matches('/').split('/').collect();
        let path_segs: Vec<&str> = path.trim_matches('/').split('/').collect();

        if pat_segs.len() != path_segs.len() {
            // Check for wildcard catch-all
            if let Some(last) = pat_segs.last() {
                if *last == "*" && path_segs.len() >= pat_segs.len() - 1 {
                    let mut params = HashMap::new();
                    for (p, s) in pat_segs.iter().zip(path_segs.iter()) {
                        if let Some(name) = p.strip_prefix(':') {
                            params.insert(name.to_string(), s.to_string());
                        } else if *p != "*" && *p != *s {
                            return None;
                        }
                    }
                    return Some(params);
                }
            }
            return None;
        }

        let mut params = HashMap::new();
        for (p, s) in pat_segs.iter().zip(path_segs.iter()) {
            if let Some(name) = p.strip_prefix(':') {
                params.insert(name.to_string(), s.to_string());
            } else if *p != *s {
                return None;
            }
        }
        Some(params)
    }

    /// Apply captured params to a destination string (replace :param with values).
    pub fn apply_params(destination: &str, params: &HashMap<String, String>) -> String {
        let mut result = destination.to_string();
        for (key, value) in params {
            result = result.replace(&format!(":{key}"), value);
        }
        result
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_match_pattern_static() {
        let result = ProjectConfig::match_pattern("/about", "/about");
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_match_pattern_no_match() {
        assert!(ProjectConfig::match_pattern("/about", "/contact").is_none());
        assert!(ProjectConfig::match_pattern("/a/b", "/a").is_none());
    }

    #[test]
    fn test_match_pattern_dynamic() {
        let result = ProjectConfig::match_pattern("/blog/:slug", "/blog/hello").unwrap();
        assert_eq!(result.get("slug").unwrap(), "hello");
    }

    #[test]
    fn test_match_pattern_multiple_params() {
        let result = ProjectConfig::match_pattern("/blog/:year/:slug", "/blog/2025/intro").unwrap();
        assert_eq!(result.get("year").unwrap(), "2025");
        assert_eq!(result.get("slug").unwrap(), "intro");
    }

    #[test]
    fn test_apply_params() {
        let mut params = HashMap::new();
        params.insert("slug".to_string(), "hello".to_string());
        assert_eq!(
            ProjectConfig::apply_params("/posts/:slug", &params),
            "/posts/hello"
        );
    }

    #[test]
    fn test_config_load_missing_file() {
        let tmp = std::env::temp_dir().join("rex_test_no_config");
        let _ = std::fs::create_dir_all(&tmp);
        let config = ProjectConfig::load(&tmp).unwrap();
        assert!(config.redirects.is_empty());
        assert!(config.rewrites.is_empty());
        assert!(config.headers.is_empty());
    }

    #[test]
    fn test_config_load_json() {
        let tmp = std::env::temp_dir().join("rex_test_config_load");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(
            tmp.join("rex.config.json"),
            r#"{
                "redirects": [
                    { "source": "/old", "destination": "/new", "permanent": true }
                ],
                "rewrites": [
                    { "source": "/api/:path", "destination": "/api/v2/:path" }
                ],
                "headers": [
                    {
                        "source": "/:path",
                        "headers": [
                            { "key": "X-Frame-Options", "value": "DENY" }
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();

        let config = ProjectConfig::load(&tmp).unwrap();
        assert_eq!(config.redirects.len(), 1);
        assert_eq!(config.redirects[0].source, "/old");
        assert_eq!(config.redirects[0].destination, "/new");
        assert!(config.redirects[0].permanent);
        assert_eq!(config.rewrites.len(), 1);
        assert_eq!(config.headers.len(), 1);
        assert_eq!(config.headers[0].headers[0].key, "X-Frame-Options");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_middleware_result_deserialize_next() {
        let json = r#"{"action":"next"}"#;
        let result: MiddlewareResult = serde_json::from_str(json).unwrap();
        assert!(matches!(result.action, MiddlewareAction::Next));
        assert!(result.url.is_none());
        assert_eq!(result.status, 307);
        assert!(result.request_headers.is_empty());
        assert!(result.response_headers.is_empty());
    }

    #[test]
    fn test_middleware_result_deserialize_redirect() {
        let json = r#"{"action":"redirect","url":"/login","status":302}"#;
        let result: MiddlewareResult = serde_json::from_str(json).unwrap();
        assert!(matches!(result.action, MiddlewareAction::Redirect));
        assert_eq!(result.url.as_deref(), Some("/login"));
        assert_eq!(result.status, 302);
    }

    #[test]
    fn test_middleware_result_deserialize_rewrite() {
        let json =
            r#"{"action":"rewrite","url":"/internal","response_headers":{"x-rewritten":"true"}}"#;
        let result: MiddlewareResult = serde_json::from_str(json).unwrap();
        assert!(matches!(result.action, MiddlewareAction::Rewrite));
        assert_eq!(result.url.as_deref(), Some("/internal"));
        assert_eq!(result.response_headers.get("x-rewritten").unwrap(), "true");
    }
}
