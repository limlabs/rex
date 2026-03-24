pub mod core;
mod core_handlers;
pub mod document;
pub mod document_rsc;
pub mod export;
pub mod handlers;
pub mod mcp;
pub mod prerender;
pub mod rex;
pub mod server;
#[cfg(feature = "build")]
pub mod startup;
pub mod state;

pub use rex::{PageResult, Rex, RexOptions};
pub use server::{RexServer, ServerConfig};
