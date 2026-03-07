#[macro_use]
mod v8_macros;

pub mod fetch;
pub mod fs;
pub mod isolate_pool;
pub mod platform;
pub mod ssr_isolate;
mod ssr_isolate_rsc;

pub use isolate_pool::IsolatePool;
pub use platform::init_v8;
pub use ssr_isolate::{RenderResult, RscRenderResult, SsrIsolate};
