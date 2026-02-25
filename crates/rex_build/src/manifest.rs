use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Maps route patterns to their client-side asset filenames
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetManifest {
    pub build_id: String,
    /// route pattern -> client chunk filename
    pub pages: HashMap<String, PageAssets>,
    /// Vendor scripts (e.g. React runtime) to load before page scripts
    #[serde(default)]
    pub vendor_scripts: Vec<String>,
    /// Client _app chunk filename (loaded before page scripts for hydration wrapping)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageAssets {
    pub js: String,
}

impl AssetManifest {
    pub fn new(build_id: String) -> Self {
        Self {
            build_id,
            pages: HashMap::new(),
            vendor_scripts: Vec::new(),
            app_script: None,
        }
    }

    pub fn add_page(&mut self, route_pattern: &str, js_filename: &str) {
        self.pages.insert(
            route_pattern.to_string(),
            PageAssets {
                js: js_filename.to_string(),
            },
        );
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}
