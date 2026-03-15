pub mod app_route;
pub mod client_manifest;
pub mod config;
pub mod error;
pub mod error_buffer;
pub mod instance;
pub mod manifest;
pub mod route;

pub use client_manifest::{ClientRefEntry, ClientReferenceManifest};
pub use config::RexConfig;
pub use error::RexError;
pub use error_buffer::{DevError, ErrorBuffer};
pub use instance::InstanceInfo;
pub use manifest::{AppRouteAssets, AssetManifest, PageAssets};
pub use route::{
    BuildConfig, DataStrategy, DevConfig, DynamicSegment, Fallback, HeaderEntry, HeaderRule,
    McpToolRoute, MiddlewareAction, MiddlewareResult, PageType, ProjectConfig, RedirectConfig,
    RedirectRule, RenderMode, RewriteRule, Route, RouteMatch, ServerSidePropsContext,
    ServerSidePropsResult,
};
