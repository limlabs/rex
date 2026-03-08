// Manifest types live in rex_core for decoupling (rex_server can use them
// without pulling in the full build toolchain). Re-exported here for
// backward compatibility.
pub use rex_core::manifest::{AppRouteAssets, AssetManifest, PageAssets};
