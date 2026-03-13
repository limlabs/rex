pub mod core;
mod core_handlers;
pub mod document;
pub mod export;
pub mod handlers;
pub mod mcp;
pub mod prerender;
pub mod rex;
pub mod server;
pub mod state;

#[cfg(feature = "build")]
pub use rex::EsmDevState;
pub use rex::{PageResult, Rex, RexOptions};
pub use server::{RexServer, ServerConfig};
