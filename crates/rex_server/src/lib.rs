pub mod core;
mod core_handlers;
pub mod document;
pub mod export;
pub mod handlers;
pub mod mcp;
pub mod prerender;
pub mod rex;
pub mod rsc_document;
pub mod server;
pub mod state;

pub use rex::{PageResult, Rex, RexOptions};
pub use server::{RexServer, ServerConfig};
