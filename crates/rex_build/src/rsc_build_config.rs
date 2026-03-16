//! Shared RSC build configuration and utilities.
//!
//! Contains helpers shared across the RSC server, SSR, and client bundle builders:
//! resolve aliases, treeshake options, common rolldown config fragments.

use crate::build_utils::runtime_server_dir;
use crate::bundler::runtime_client_dir;
use anyhow::Result;
use rex_core::RexConfig;
use std::path::PathBuf;
use tracing::debug;

/// Shared context threaded through all RSC bundle builds.
pub struct RscBuildContext<'a> {
    pub config: &'a RexConfig,
    /// Canonicalized project root (computed once).
    pub project_root: PathBuf,
    pub build_id: &'a str,
    pub define: &'a [(String, String)],
    pub module_dirs: &'a [String],
    /// Pre-compiled MDX file aliases (original path → compiled JSX path).
    pub mdx_aliases: Vec<(String, Vec<Option<String>>)>,
}

impl<'a> RscBuildContext<'a> {
    pub fn new(
        config: &'a RexConfig,
        build_id: &'a str,
        define: &'a [(String, String)],
        module_dirs: &'a [String],
    ) -> Self {
        let project_root = config.project_root.canonicalize().unwrap_or_else(|e| {
            debug!(
                path = %config.project_root.display(),
                error = %e,
                "Failed to canonicalize project root, using original path"
            );
            config.project_root.clone()
        });
        // Pre-compile MDX content files for rolldown alias resolution
        let mdx_aliases =
            crate::mdx::compile_all_mdx_files(&project_root, &config.server_build_dir())
                .unwrap_or_default();

        Self {
            config,
            project_root,
            build_id,
            define,
            module_dirs,
            mdx_aliases,
        }
    }

    /// Short hash prefix of the build ID for chunk filenames.
    pub fn hash(&self) -> &str {
        &self.build_id[..8.min(self.build_id.len())]
    }

    /// Empty module type map for non-JS assets (shared across all 3 bundles).
    ///
    /// These file types cannot be parsed as JavaScript by rolldown/OXC.
    /// They are treated as empty modules so bundling can proceed without
    /// crashing on transitive imports from node_modules.
    pub fn non_js_empty_module_types(&self) -> rustc_hash::FxHashMap<String, rolldown::ModuleType> {
        let mut m = rustc_hash::FxHashMap::default();
        // Text-based non-JS assets → empty
        for ext in &[".css", ".scss", ".sass", ".less", ".mdx", ".svg"] {
            m.insert((*ext).to_string(), rolldown::ModuleType::Empty);
        }
        // Binary assets → binary (prevents UTF-8 read errors)
        for ext in &[
            ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".avif", ".bmp", ".tiff", ".woff",
            ".woff2", ".ttf", ".eot", ".wasm",
        ] {
            m.insert((*ext).to_string(), rolldown::ModuleType::Binary);
        }
        m
    }

    /// Minify options based on dev/prod mode.
    pub fn minify_options(&self) -> Option<rolldown_common::RawMinifyOptions> {
        if !self.config.dev {
            Some(rolldown_common::RawMinifyOptions::Bool(true))
        } else {
            None
        }
    }

    /// V8 polyfills + webpack shims banner for server-side bundles.
    pub fn server_banner(&self) -> String {
        let webpack_shims = include_str!("../../../runtime/rsc/webpack-shims.ts");
        format!("{}\n{}", crate::bundler::V8_POLYFILLS, webpack_shims)
    }
}

/// Build rolldown resolve aliases for `rex/*` and `next/*` built-in imports (client bundles).
///
/// Maps `rex/link`, `rex/head`, `rex/router`, `rex/image` and their
/// `next/*` equivalents to runtime files in `runtime/client/`, and
/// `rex/actions` to `runtime/server/actions`.
pub(crate) fn build_rex_aliases() -> Result<Vec<(String, Vec<Option<String>>)>> {
    let client_dir = runtime_client_dir()?;
    let mut aliases = Vec::new();

    let mappings = [
        ("rex/link", "link"),
        ("rex/head", "head"),
        ("rex/router", "use-router"),
        ("rex/image", "image"),
        ("next/link", "link"),
        ("next/head", "head"),
        ("next/router", "use-router"),
        ("next/image", "image"),
    ];

    for (specifier, file_stem) in &mappings {
        for ext in &["ts", "tsx", "js", "jsx"] {
            let candidate = client_dir.join(format!("{file_stem}.{ext}"));
            if candidate.exists() {
                aliases.push((
                    specifier.to_string(),
                    vec![Some(candidate.to_string_lossy().to_string())],
                ));
                break;
            }
        }
    }

    // Server-side aliases (rex/actions → runtime/server/actions.ts)
    let server_dir = runtime_server_dir()?;
    let server_mappings = [("rex/actions", "actions")];
    for (specifier, file_stem) in &server_mappings {
        for ext in &["ts", "tsx", "js", "jsx"] {
            let candidate = server_dir.join(format!("{file_stem}.{ext}"));
            if candidate.exists() {
                aliases.push((
                    specifier.to_string(),
                    vec![Some(candidate.to_string_lossy().to_string())],
                ));
                break;
            }
        }
    }

    Ok(aliases)
}

/// Build rolldown resolve aliases for RSC server bundles.
///
/// Same as [`build_rex_aliases`] but overrides `rex/link` and `rex/head`
/// with server-side stubs from `runtime/server/` that don't contain
/// event handlers (which are rejected by React's flight protocol).
pub(crate) fn build_rex_server_aliases() -> Result<Vec<(String, Vec<Option<String>>)>> {
    let mut aliases = build_rex_aliases()?;
    let server_dir = runtime_server_dir()?;

    let server_overrides = [
        ("rex/link", "link"),
        ("rex/head", "head"),
        ("next/link", "link"),
        ("next/head", "head"),
    ];
    for (specifier, file_stem) in &server_overrides {
        for ext in &["ts", "tsx", "js", "jsx"] {
            let candidate = server_dir.join(format!("{file_stem}.{ext}"));
            if candidate.exists() {
                // Remove the existing client alias and add server one
                aliases.retain(|(s, _)| s != *specifier);
                aliases.push((
                    specifier.to_string(),
                    vec![Some(candidate.to_string_lossy().to_string())],
                ));
                break;
            }
        }
    }

    Ok(aliases)
}

/// Tree-shake options that mark React packages as side-effect-free.
///
/// Allows rolldown to aggressively eliminate unused exports from
/// `node_modules/react*`. React's production builds use `@__PURE__`
/// annotations which rolldown respects when `annotations: true`.
pub(crate) fn react_treeshake_options() -> rolldown_common::TreeshakeOptions {
    rolldown_common::TreeshakeOptions::Option(rolldown_common::InnerOptions {
        module_side_effects: rolldown_common::ModuleSideEffects::Rules(vec![
            rolldown_common::ModuleSideEffectsRule {
                test: Some(
                    rolldown_utils::js_regex::HybridRegex::new("node_modules[\\\\/]react")
                        .expect("valid regex"),
                ),
                external: None,
                side_effects: false,
            },
        ]),
        annotations: Some(true),
        ..Default::default()
    })
}

/// Sanitize a file path into a valid chunk name.
pub(crate) fn sanitize_filename(path: &str) -> String {
    path.replace(['/', '\\', '.'], "_")
        .trim_matches('_')
        .to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_path_components() {
        assert_eq!(
            sanitize_filename("components/Counter.tsx"),
            "components_Counter_tsx"
        );
        assert_eq!(sanitize_filename("app/page.tsx"), "app_page_tsx");
    }

    #[test]
    fn sanitize_empty_string() {
        assert_eq!(sanitize_filename(""), "");
    }

    #[test]
    fn sanitize_only_separators() {
        assert_eq!(sanitize_filename("///"), "");
        assert_eq!(sanitize_filename("..."), "");
    }

    #[test]
    fn sanitize_windows_path() {
        assert_eq!(
            sanitize_filename("components\\Counter.tsx"),
            "components_Counter_tsx"
        );
    }

    #[test]
    fn sanitize_consecutive_separators() {
        assert_eq!(sanitize_filename("a//b..c"), "a__b__c");
    }

    #[test]
    fn build_rex_aliases_finds_runtime_files() {
        let aliases = build_rex_aliases().unwrap();
        // Should find at least rex/link and rex/head
        let specs: Vec<&str> = aliases.iter().map(|(s, _)| s.as_str()).collect();
        assert!(specs.contains(&"rex/link"), "missing rex/link alias");
        assert!(specs.contains(&"rex/head"), "missing rex/head alias");
    }

    #[test]
    fn react_treeshake_options_valid() {
        // Should not panic
        let _opts = react_treeshake_options();
    }

    #[test]
    fn rsc_build_context_hash() {
        let config = rex_core::RexConfig::new(std::path::PathBuf::from("/tmp")).with_dev(true);
        let ctx = RscBuildContext::new(&config, "abcdef1234567890", &[], &[]);
        assert_eq!(ctx.hash(), "abcdef12");
    }

    #[test]
    fn rsc_build_context_short_build_id() {
        let config = rex_core::RexConfig::new(std::path::PathBuf::from("/tmp")).with_dev(true);
        let ctx = RscBuildContext::new(&config, "abc", &[], &[]);
        assert_eq!(ctx.hash(), "abc");
    }

    #[test]
    fn non_js_empty_module_types_has_expected_extensions() {
        let config = rex_core::RexConfig::new(std::path::PathBuf::from("/tmp")).with_dev(true);
        let ctx = RscBuildContext::new(&config, "test", &[], &[]);
        let types = ctx.non_js_empty_module_types();
        assert!(types.contains_key(".css"));
        assert!(types.contains_key(".scss"));
        assert!(types.contains_key(".mdx"));
        assert!(types.contains_key(".svg"));
    }

    #[test]
    fn minify_options_dev_vs_prod() {
        let dev_config = rex_core::RexConfig::new(std::path::PathBuf::from("/tmp")).with_dev(true);
        let dev_ctx = RscBuildContext::new(&dev_config, "test", &[], &[]);
        assert!(dev_ctx.minify_options().is_none());

        let prod_config =
            rex_core::RexConfig::new(std::path::PathBuf::from("/tmp")).with_dev(false);
        let prod_ctx = RscBuildContext::new(&prod_config, "test", &[], &[]);
        assert!(prod_ctx.minify_options().is_some());
    }
}
