//! MDX-to-JSX compiler for Rex.
//!
//! Compiles MDX source strings into JSX modules by parsing markdown + JSX
//! with the `markdown` crate and generating `React.createElement` calls.

mod compiler;
mod helpers;

pub use compiler::{compile_mdx_with_options, MdxOptions};
pub use helpers::{extract_esm, find_mdx_components, yaml_to_js_object};
