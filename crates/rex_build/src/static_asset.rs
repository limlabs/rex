//! Rolldown plugin for static asset imports (.png, .jpg, etc.).
//!
//! When a source file imports a binary asset (e.g. `import bg from './bg.png'`),
//! this plugin:
//!   1. Copies the file to `{client_dir}/assets/{name}-{hash}.{ext}`
//!   2. Returns a JS module exporting `{ src, height, width }` (Next.js compatible)
//!
//! The copied files are served by Rex's static file server at
//! `/_rex/static/assets/{name}-{hash}.{ext}`.

use std::path::{Path, PathBuf};

/// Image/binary file extensions handled by this plugin.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "avif", "bmp", "tiff", "svg",
];

#[derive(Debug)]
pub struct StaticAssetPlugin {
    /// Directory to copy assets into (typically `{output}/client/assets/`).
    asset_dir: PathBuf,
}

impl StaticAssetPlugin {
    pub fn new(asset_dir: PathBuf) -> Self {
        Self { asset_dir }
    }

    fn is_image_file(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        IMAGE_EXTENSIONS
            .iter()
            .any(|ext| lower.ends_with(&format!(".{ext}")))
    }

    /// Compute a short hex hash from file contents (FNV-1a).
    fn content_hash(data: &[u8]) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in data {
            h ^= b as u64;
            h = h.wrapping_mul(0x00000100000001B3);
        }
        format!("{h:016x}")
    }
}

impl rolldown::plugin::Plugin for StaticAssetPlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("rex:static-asset")
    }

    fn register_hook_usage(&self) -> rolldown::plugin::HookUsage {
        rolldown::plugin::HookUsage::Load
    }

    fn load(
        &self,
        _ctx: rolldown::plugin::SharedLoadPluginContext,
        args: &rolldown::plugin::HookLoadArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookLoadReturn> + Send {
        let id = args.id;
        let result = if Self::is_image_file(id) {
            load_asset(id, &self.asset_dir)
        } else {
            None
        };
        async { Ok(result) }
    }
}

/// Read an asset file, copy it to the output dir, return a JS module.
fn load_asset(id: &str, asset_dir: &Path) -> Option<rolldown::plugin::HookLoadOutput> {
    let src_path = Path::new(id);
    if !src_path.exists() {
        return None;
    }

    let data = std::fs::read(src_path).ok()?;
    let hash = &StaticAssetPlugin::content_hash(&data)[..8];
    let stem = src_path.file_stem()?.to_string_lossy();
    let ext = src_path.extension()?.to_string_lossy();
    let out_name = format!("{stem}-{hash}.{ext}");

    // Copy to asset output directory
    let _ = std::fs::create_dir_all(asset_dir);
    let dest = asset_dir.join(&out_name);
    if !dest.exists() {
        let _ = std::fs::write(&dest, &data);
    }

    let url = format!("/_rex/static/assets/{out_name}");

    // Next.js static image import shape: { src, height, width }
    let code = format!("export default {{ src: \"{url}\", height: 0, width: 0 }};\n");

    Some(rolldown::plugin::HookLoadOutput {
        code: code.into(),
        module_type: Some(rolldown::ModuleType::Js),
        ..Default::default()
    })
}
