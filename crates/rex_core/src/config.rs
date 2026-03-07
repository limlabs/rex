use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RexConfig {
    pub project_root: PathBuf,
    pub pages_dir: PathBuf,
    pub app_dir: PathBuf,
    pub output_dir: PathBuf,
    pub port: u16,
    pub dev: bool,
}

impl RexConfig {
    pub fn new(project_root: PathBuf) -> Self {
        let pages_dir = project_root.join("pages");
        let app_dir = project_root.join("app");
        let output_dir = project_root.join(".rex");
        Self {
            project_root,
            pages_dir,
            app_dir,
            output_dir,
            port: 3000,
            dev: false,
        }
    }

    /// Whether this project has an app/ directory (RSC/App Router).
    pub fn has_app_dir(&self) -> bool {
        self.app_dir.exists()
    }

    /// Whether this project has a pages/ directory (Pages Router).
    pub fn has_pages_dir(&self) -> bool {
        self.pages_dir.exists()
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

    /// Check that at least one of pages/ or app/ directories exists.
    pub fn validate(&self) -> Result<(), crate::RexError> {
        if !self.pages_dir.exists() && !self.app_dir.exists() {
            return Err(crate::RexError::Config(format!(
                "Neither pages/ nor app/ directory found in {}",
                self.project_root.display()
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_new_sets_defaults() {
        let cfg = RexConfig::new(PathBuf::from("/app"));
        assert_eq!(cfg.project_root, PathBuf::from("/app"));
        assert_eq!(cfg.pages_dir, PathBuf::from("/app/pages"));
        assert_eq!(cfg.app_dir, PathBuf::from("/app/app"));
        assert_eq!(cfg.output_dir, PathBuf::from("/app/.rex"));
        assert_eq!(cfg.port, 3000);
        assert!(!cfg.dev);
    }

    #[test]
    fn test_with_dev_and_port() {
        let cfg = RexConfig::new(PathBuf::from("/app"))
            .with_dev(true)
            .with_port(8080);
        assert!(cfg.dev);
        assert_eq!(cfg.port, 8080);
    }

    #[test]
    fn test_build_dirs() {
        let cfg = RexConfig::new(PathBuf::from("/app"));
        assert_eq!(
            cfg.server_build_dir(),
            PathBuf::from("/app/.rex/build/server")
        );
        assert_eq!(
            cfg.client_build_dir(),
            PathBuf::from("/app/.rex/build/client")
        );
        assert_eq!(
            cfg.server_bundle_path(),
            PathBuf::from("/app/.rex/build/server/server-bundle.js")
        );
        assert_eq!(
            cfg.manifest_path(),
            PathBuf::from("/app/.rex/build/manifest.json")
        );
    }

    #[test]
    fn test_validate_missing_dirs() {
        let cfg = RexConfig::new(PathBuf::from("/nonexistent"));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_with_pages_dir() {
        let tmp = std::env::temp_dir().join("rex_test_config_validate");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("pages")).unwrap();
        let cfg = RexConfig::new(tmp.clone());
        assert!(cfg.validate().is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_has_dirs() {
        let tmp = std::env::temp_dir().join("rex_test_config_has_dirs");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let cfg = RexConfig::new(tmp.clone());
        assert!(!cfg.has_pages_dir());
        assert!(!cfg.has_app_dir());

        std::fs::create_dir(tmp.join("pages")).unwrap();
        assert!(cfg.has_pages_dir());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
