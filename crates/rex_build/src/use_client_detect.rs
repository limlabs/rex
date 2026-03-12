//! Rolldown plugin to detect `"use client"` directives in node_modules.
//!
//! The RSC module graph walker only follows relative imports, so `"use client"`
//! components in node_modules (e.g. PayloadCMS UI, Radix, Lexical) are not
//! detected as client boundaries. This plugin intercepts the `load` hook and
//! replaces such modules with no-op component stubs that render `null`, preventing
//! the RSC flight serializer from encountering event handlers and hooks.

use std::fs;

/// Rolldown plugin that detects `"use client"` in node_modules files and
/// replaces them with no-op stubs for the RSC server bundle.
#[derive(Debug)]
pub(crate) struct UseClientDetectPlugin;

impl UseClientDetectPlugin {
    pub fn new() -> Self {
        Self
    }

    /// Check if a file's source starts with a `"use client"` directive.
    fn has_use_client_directive(source: &str) -> bool {
        for line in source.lines() {
            let trimmed = line.trim();
            // Skip empty lines and comments
            if trimmed.is_empty()
                || trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
            {
                continue;
            }
            // Check for "use client" directive (with single or double quotes).
            // Use starts_with rather than exact match because minified files
            // (e.g. PayloadCMS, Radix) put imports on the same line:
            //   "use client";import{a as ge,...}from"./chunk.js";
            if trimmed.starts_with("\"use client\"") || trimmed.starts_with("'use client'") {
                return true;
            }
            // If we hit any other statement, stop looking (directives must be first)
            return false;
        }
        false
    }

    /// Check if a file path is inside node_modules.
    fn is_node_modules(path: &str) -> bool {
        path.contains("/node_modules/") || path.contains("\\node_modules\\")
    }

    /// Extract export names from source (reuses HeavyPackageStubPlugin's logic).
    fn extract_exports(source: &str) -> Vec<String> {
        crate::server_bundle::HeavyPackageStubPlugin::extract_exports(source)
    }

    /// Generate a stub where each export is a function that returns null.
    /// These are valid React components that simply render nothing.
    fn generate_noop_stub(exports: &[String]) -> String {
        let mut code = String::from("var __$N = function() { return null; };\n");
        for name in exports {
            if name == "default" {
                code.push_str("export default __$N;\n");
            } else {
                code.push_str(&format!("export var {name} = __$N;\n"));
            }
        }
        // Always provide a default export
        if !exports.contains(&"default".to_string()) {
            code.push_str("export default __$N;\n");
        }
        code
    }
}

impl rolldown::plugin::Plugin for UseClientDetectPlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("rex:use-client-detect")
    }

    fn load(
        &self,
        _ctx: rolldown::plugin::SharedLoadPluginContext,
        args: &rolldown::plugin::HookLoadArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookLoadReturn> + Send {
        let result = if Self::is_node_modules(args.id) {
            if let Ok(source) = fs::read_to_string(args.id) {
                if Self::has_use_client_directive(&source) {
                    let exports = Self::extract_exports(&source);
                    let code = Self::generate_noop_stub(&exports);
                    Some(rolldown::plugin::HookLoadOutput {
                        code: code.into(),
                        ..Default::default()
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        async move { Ok(result) }
    }

    fn register_hook_usage(&self) -> rolldown::plugin::HookUsage {
        rolldown::plugin::HookUsage::Load
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_double_quote_directive() {
        assert!(UseClientDetectPlugin::has_use_client_directive(
            "\"use client\";\nexport default function() {}"
        ));
    }

    #[test]
    fn detects_single_quote_directive() {
        assert!(UseClientDetectPlugin::has_use_client_directive(
            "'use client';\nexport default function() {}"
        ));
    }

    #[test]
    fn detects_directive_without_semicolon() {
        assert!(UseClientDetectPlugin::has_use_client_directive(
            "\"use client\"\nexport default function() {}"
        ));
    }

    #[test]
    fn skips_comments_before_directive() {
        assert!(UseClientDetectPlugin::has_use_client_directive(
            "// comment\n\"use client\";\nexport default function() {}"
        ));
    }

    #[test]
    fn rejects_non_directive() {
        assert!(!UseClientDetectPlugin::has_use_client_directive(
            "export default function() {}"
        ));
    }

    #[test]
    fn rejects_directive_after_import() {
        assert!(!UseClientDetectPlugin::has_use_client_directive(
            "import React from 'react';\n\"use client\";"
        ));
    }

    #[test]
    fn detects_minified_directive() {
        // Minified packages put "use client"; on the same line as imports
        assert!(UseClientDetectPlugin::has_use_client_directive(
            "\"use client\";import{a as ge}from\"./chunk.js\";"
        ));
    }

    #[test]
    fn detects_minified_single_quote_directive() {
        assert!(UseClientDetectPlugin::has_use_client_directive(
            "'use client';import{a as ge}from'./chunk.js';"
        ));
    }

    #[test]
    fn generates_noop_stub() {
        let stub = UseClientDetectPlugin::generate_noop_stub(&[
            "default".to_string(),
            "Button".to_string(),
        ]);
        assert!(stub.contains("export default __$N;"));
        assert!(stub.contains("export var Button = __$N;"));
    }
}
