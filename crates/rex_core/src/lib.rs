pub mod config;
pub mod error;
pub mod route;

pub use config::RexConfig;
pub use error::RexError;
pub use route::{
    DataStrategy, DynamicSegment, PageType, RedirectConfig, Route, RouteMatch,
    ServerSidePropsContext, ServerSidePropsResult, StaticPropsContext, StaticPropsResult,
};
