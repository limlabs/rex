pub mod builtin_modules;
pub mod bundler;
pub mod client_manifest;
pub mod dce;
pub mod embedded_runtime;
pub mod manifest;
pub mod rsc_bundler;
pub mod rsc_graph;
pub mod server_action_manifest;

// Internal modules extracted from bundler.rs
pub(crate) mod build_utils;
pub(crate) mod client_bundle;
pub(crate) mod css_collect;
pub(crate) mod css_modules;
pub(crate) mod mdx;
pub(crate) mod server_bundle;
pub mod tailwind;

// Internal modules extracted from rsc_bundler.rs
pub(crate) mod rsc_build_config;
pub(crate) mod rsc_client_bundle;
pub(crate) mod rsc_entries;
pub(crate) mod rsc_server_bundle;
pub(crate) mod rsc_ssr_bundle;
pub(crate) mod rsc_stubs;

pub use bundler::{build_bundles, V8_POLYFILLS};
pub use manifest::AssetManifest;
pub use tailwind::{
    collect_all_css_import_paths, find_tailwind_bin, needs_tailwind, process_tailwind_css,
};
