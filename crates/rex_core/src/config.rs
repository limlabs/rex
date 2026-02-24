use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RexConfig {
    pub project_root: PathBuf,
    pub pages_dir: PathBuf,
    pub output_dir: PathBuf,
    pub port: u16,
    pub dev: bool,
}

impl RexConfig {
    pub fn new(project_root: PathBuf) -> Self {
        let pages_dir = project_root.join("pages");
        let output_dir = project_root.join(".rex");
        Self {
            project_root,
            pages_dir,
            output_dir,
            port: 3000,
            dev: false,
        }
    }

    pub fn with_dev(mut self, dev: bool) -> Self {
        self.dev = dev;
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn server_build_dir(&self) -> PathBuf {
        self.output_dir.join("build").join("server")
    }

    pub fn client_build_dir(&self) -> PathBuf {
        self.output_dir.join("build").join("client")
    }

    pub fn server_bundle_path(&self) -> PathBuf {
        self.server_build_dir().join("server-bundle.js")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.output_dir.join("build").join("manifest.json")
    }

    pub fn node_modules_dir(&self) -> PathBuf {
        self.project_root.join("node_modules")
    }

    /// Resolve a path relative to the project root
    pub fn resolve(&self, path: &str) -> PathBuf {
        self.project_root.join(path)
    }

    /// Check if the pages directory exists
    pub fn validate(&self) -> Result<(), crate::RexError> {
        if !self.pages_dir.exists() {
            return Err(crate::RexError::Config(format!(
                "Pages directory not found: {}",
                self.pages_dir.display()
            )));
        }
        Ok(())
    }
}

impl Default for RexConfig {
    fn default() -> Self {
        Self::new(PathBuf::from("."))
    }
}
