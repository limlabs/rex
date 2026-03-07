pub mod core;
pub mod document;
pub mod handlers;
pub mod mcp;
pub mod rex;
pub mod server;
pub mod state;

pub use rex::{PageResult, Rex, RexOptions};
pub use server::{RexServer, ServerConfig};
