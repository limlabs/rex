pub mod builtin_modules;
pub mod bundler;
pub mod client_dep_bundle;
pub mod client_manifest;
pub mod dce;
pub(crate) mod diagnostics;
pub mod embedded_runtime;
pub(crate) mod esm_parse_helpers;
pub mod esm_transform;
pub mod extra_dep_bundle;
pub mod manifest;
pub mod precomputed_ids;
pub mod rsc_bundler;
pub mod rsc_graph;
pub mod server_action_extract;
pub mod server_action_manifest;
pub mod server_dep_bundle;

// Internal modules extracted from bundler.rs
pub mod build_utils;
pub(crate) mod client_bundle;
pub(crate) mod css_collect;
pub(crate) mod css_modules;
pub(crate) mod font;
pub(crate) mod font_google;
pub mod mdx;
pub(crate) mod page_exports;
pub mod server_bundle;
pub mod tailwind;
pub mod tailwind_optimize;
pub(crate) mod tailwind_scan;
pub(crate) mod tailwind_v8;

// Internal modules extracted from rsc_bundler.rs
pub(crate) mod cjs_interop;
pub mod rsc_build_config;
pub mod rsc_client_bundle;
pub mod rsc_entries;
pub(crate) mod rsc_server_bundle;
pub(crate) mod rsc_ssr_bundle;
pub mod rsc_stubs;
pub(crate) mod static_asset;
pub(crate) mod use_client_detect;

pub use bundler::{build_bundles, build_bundles_with_id, resolve_modules_dirs, V8_POLYFILLS};
pub use manifest::AssetManifest;
pub use tailwind::{
    collect_all_css_import_paths, find_tailwind_bin, needs_tailwind, process_tailwind_css,
};
