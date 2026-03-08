//! Types for the App Router (`app/` directory).

use crate::DynamicSegment;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A segment in the app directory tree.
///
/// Each directory under `app/` produces a segment, which may contain a
/// `page.tsx`, `layout.tsx`, `loading.tsx`, `error.tsx`, and/or `not-found.tsx`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSegment {
    /// Directory segment name, e.g. "blog", "[slug]", "(marketing)"
    pub segment: String,
    /// Layout component for this segment
    pub layout: Option<PathBuf>,
    /// Page component (makes this segment a routable endpoint)
    pub page: Option<PathBuf>,
    /// Loading fallback (Suspense boundary)
    pub loading: Option<PathBuf>,
    /// Error boundary component
    pub error_boundary: Option<PathBuf>,
    /// Not-found component
    pub not_found: Option<PathBuf>,
    /// Child segments
    pub children: Vec<AppSegment>,
}

/// A flattened route derived from the app directory tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppRoute {
    /// URL pattern, e.g. "/blog/:slug"
    pub pattern: String,
    /// Absolute path to the page component
    pub page_path: PathBuf,
    /// Layout chain from root to nearest layout (ordered root-first)
    pub layout_chain: Vec<PathBuf>,
    /// Loading boundaries parallel to layout_chain (None if no loading at that level)
    pub loading_chain: Vec<Option<PathBuf>>,
    /// Error boundaries parallel to layout_chain
    pub error_chain: Vec<Option<PathBuf>>,
    /// Dynamic segments extracted from the pattern
    pub dynamic_segments: Vec<DynamicSegment>,
    /// Specificity score for route matching priority
    pub specificity: u32,
}

/// Result of scanning the `app/` directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppScanResult {
    /// The segment tree rooted at `app/`
    pub root: AppSegment,
    /// Flattened routes with resolved layout chains
    pub routes: Vec<AppRoute>,
    /// Path to the root layout. `None` when route groups each provide their own layout.
    pub root_layout: Option<PathBuf>,
}

impl AppRoute {
    /// Convert to a [`crate::Route`] for use with `RouteTrie`.
    pub fn to_route(&self) -> crate::Route {
        crate::Route {
            pattern: self.pattern.clone(),
            file_path: self
                .page_path
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_default(),
            abs_path: self.page_path.clone(),
            dynamic_segments: self.dynamic_segments.clone(),
            page_type: crate::PageType::Regular,
            specificity: self.specificity,
        }
    }
}

impl AppScanResult {
    /// Convert all app routes to `Route` objects for building a `RouteTrie`.
    pub fn to_routes(&self) -> Vec<crate::Route> {
        self.routes.iter().map(|r| r.to_route()).collect()
    }
}
