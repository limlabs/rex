use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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

/// How a page fetches its data
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataStrategy {
    /// No data fetching — rendered with empty props
    None,
    /// getStaticProps — pre-rendered at build time
    StaticProps,
    /// getServerSideProps — rendered per-request
    ServerSideProps,
}

/// Context passed to getStaticProps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticPropsContext {
    pub params: HashMap<String, String>,
}

/// Result from getStaticProps (same shape as GSSP result, plus revalidate)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StaticPropsResult {
    Props {
        props: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revalidate: Option<u64>,
    },
    Redirect {
        redirect: RedirectConfig,
    },
    NotFound {
        #[serde(rename = "notFound")]
        not_found: bool,
    },
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
