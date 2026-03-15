//! Integration tests for RSC context provider patterns.
//!
//! Validates that `"use client"` context providers and consumers are correctly
//! detected as client boundaries, generate proper client reference stubs,
//! and appear in the client manifest.

#![allow(clippy::unwrap_used)]

use rex_build::rsc_graph::analyze_module_graph;
use std::fs;

fn setup_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Create a temp project with a context provider pattern:
/// - ThemeProvider.tsx ("use client") — exports ThemeContext, ThemeProvider, useTheme
/// - ThemeDisplay.tsx ("use client") — imports useTheme, renders theme value
/// - page.tsx (server) — imports both, wraps ThemeDisplay in ThemeProvider
fn create_context_project(root: &std::path::Path) {
    let comp_dir = root.join("components");
    fs::create_dir_all(&comp_dir).unwrap();

    fs::write(
        comp_dir.join("ThemeProvider.tsx"),
        r#"
"use client";
import React, { createContext, useContext, useState } from 'react';

export const ThemeContext = createContext({ theme: 'light', setTheme: () => {} });
export function useTheme() { return useContext(ThemeContext); }
export function ThemeProvider({ initialTheme, children }) {
  const [theme, setTheme] = useState(initialTheme);
  return React.createElement(ThemeContext.Provider, { value: { theme, setTheme } }, children);
}
export default ThemeProvider;
"#,
    )
    .unwrap();

    fs::write(
        comp_dir.join("ThemeDisplay.tsx"),
        r#"
"use client";
import React from 'react';
import { useTheme } from './ThemeProvider';

export default function ThemeDisplay() {
  const { theme } = useTheme();
  return React.createElement('span', null, 'Current theme: ' + theme);
}
"#,
    )
    .unwrap();

    let app_dir = root.join("app");
    fs::create_dir_all(&app_dir).unwrap();

    fs::write(
        app_dir.join("page.tsx"),
        r#"
import { ThemeProvider } from '../components/ThemeProvider';
import ThemeDisplay from '../components/ThemeDisplay';

export default function Page() {
  return React.createElement(ThemeProvider, { initialTheme: 'dark' },
    React.createElement(ThemeDisplay, null)
  );
}
"#,
    )
    .unwrap();
}

#[test]
fn context_provider_detected_as_client_boundary() {
    let dir = setup_temp_dir();
    let root = dir.path();
    create_context_project(root);

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let client_boundaries = graph.client_boundary_modules();

    // Both ThemeProvider and ThemeDisplay should be client boundaries
    let provider_found = client_boundaries
        .iter()
        .any(|m| m.path.ends_with("ThemeProvider.tsx"));
    let display_found = client_boundaries
        .iter()
        .any(|m| m.path.ends_with("ThemeDisplay.tsx"));

    assert!(
        provider_found,
        "ThemeProvider.tsx should be detected as client boundary. Boundaries: {:?}",
        client_boundaries
            .iter()
            .map(|m| m.path.display().to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        display_found,
        "ThemeDisplay.tsx should be detected as client boundary. Boundaries: {:?}",
        client_boundaries
            .iter()
            .map(|m| m.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn context_provider_exports_captured() {
    let dir = setup_temp_dir();
    let root = dir.path();
    create_context_project(root);

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // Find ThemeProvider module
    let provider = graph
        .modules
        .values()
        .find(|m| m.path.ends_with("ThemeProvider.tsx"))
        .expect("ThemeProvider should be in graph");

    assert!(provider.is_client);
    assert!(
        provider.exports.contains(&"ThemeContext".to_string()),
        "Should export ThemeContext, got: {:?}",
        provider.exports
    );
    assert!(
        provider.exports.contains(&"useTheme".to_string()),
        "Should export useTheme, got: {:?}",
        provider.exports
    );
    assert!(
        provider.exports.contains(&"ThemeProvider".to_string()),
        "Should export ThemeProvider, got: {:?}",
        provider.exports
    );
    assert!(
        provider.exports.contains(&"default".to_string()),
        "Should export default, got: {:?}",
        provider.exports
    );
}

#[test]
fn context_consumer_detected_separately() {
    let dir = setup_temp_dir();
    let root = dir.path();
    create_context_project(root);

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // ThemeDisplay is a separate client boundary
    let display = graph
        .modules
        .values()
        .find(|m| m.path.ends_with("ThemeDisplay.tsx"))
        .expect("ThemeDisplay should be in graph");

    assert!(display.is_client);
    assert!(
        display.exports.contains(&"default".to_string()),
        "Should export default, got: {:?}",
        display.exports
    );
}

#[test]
fn context_server_page_is_not_client() {
    let dir = setup_temp_dir();
    let root = dir.path();
    create_context_project(root);

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let page = graph
        .modules
        .values()
        .find(|m| m.path.ends_with("page.tsx"))
        .expect("page.tsx should be in graph");

    assert!(
        !page.is_client,
        "page.tsx should be a server component, not a client component"
    );
}

#[test]
fn context_graph_does_not_recurse_into_client_deps() {
    let dir = setup_temp_dir();
    let root = dir.path();
    create_context_project(root);

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // The graph should contain: page.tsx (server), ThemeProvider.tsx (client), ThemeDisplay.tsx (client)
    // ThemeDisplay imports from ThemeProvider, but since both are client boundaries,
    // ThemeProvider's imports should not be followed further
    let module_count = graph.modules.len();

    // We expect exactly 3 modules: page, ThemeProvider, ThemeDisplay
    assert_eq!(
        module_count,
        3,
        "Expected 3 modules (page + 2 client boundaries), got {}. Modules: {:?}",
        module_count,
        graph
            .modules
            .keys()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn context_client_stubs_generated_correctly() {
    use rex_build::rsc_stubs::generate_client_stub;

    // Simulate what the RSC bundler does for a context provider
    let exports = vec![
        "ThemeContext".to_string(),
        "useTheme".to_string(),
        "ThemeProvider".to_string(),
        "default".to_string(),
    ];

    let stub = generate_client_stub("components/ThemeProvider.tsx", &exports, "test-build");

    // Every export should have a client reference object
    assert!(
        stub.contains("react.client.reference"),
        "Stub should contain client reference markers"
    );
    assert!(
        stub.contains("$$name: \"ThemeContext\""),
        "Stub should have ThemeContext reference"
    );
    assert!(
        stub.contains("$$name: \"useTheme\""),
        "Stub should have useTheme reference"
    );
    assert!(
        stub.contains("$$name: \"ThemeProvider\""),
        "Stub should have ThemeProvider reference"
    );
    assert!(
        stub.contains("$$name: \"default\""),
        "Stub should have default reference"
    );
    assert!(
        stub.contains("export default"),
        "Stub should have default export"
    );
}

#[test]
fn context_client_manifest_populated() {
    use rex_build::client_manifest::{client_reference_id, ClientReferenceManifest};

    // Simulate manifest population for a context provider
    let rel_path = "components/ThemeProvider.tsx";
    let build_id = "test-build";
    let exports = vec![
        "ThemeContext".to_string(),
        "useTheme".to_string(),
        "ThemeProvider".to_string(),
        "default".to_string(),
    ];

    let mut manifest = ClientReferenceManifest::new();
    for export in &exports {
        let ref_id = client_reference_id(rel_path, export, build_id);
        manifest.add(&ref_id, "/chunk.js".to_string(), export.clone());
    }

    // All exports should be in the manifest
    assert_eq!(
        manifest.entries.len(),
        4,
        "Manifest should have 4 entries for context provider"
    );

    // Verify each entry has the correct export name
    let export_names: Vec<_> = manifest
        .entries
        .values()
        .map(|e| e.export_name.as_str())
        .collect();
    assert!(export_names.contains(&"ThemeContext"));
    assert!(export_names.contains(&"useTheme"));
    assert!(export_names.contains(&"ThemeProvider"));
    assert!(export_names.contains(&"default"));
}

#[test]
fn node_modules_use_client_provider_detected() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Create a mock node_modules package with a "use client" provider
    let pkg_dir = root.join("node_modules/mock-ui");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.json"),
        r#"{"name":"mock-ui","version":"1.0.0","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(
        pkg_dir.join("index.js"),
        r#"
"use client";
export function MockProvider({ children }) { return children; }
export function useMockContext() { return {}; }
export default MockProvider;
"#,
    )
    .unwrap();

    // Server page imports from the node_modules package
    let app_dir = root.join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("page.tsx"),
        r#"
import { MockProvider } from 'mock-ui';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // The node_modules provider should be detected as a client boundary
    let client_boundaries = graph.client_boundary_modules();
    let mock_provider = client_boundaries
        .iter()
        .find(|m| m.path.to_string_lossy().contains("mock-ui"));

    assert!(
        mock_provider.is_some(),
        "mock-ui provider should be detected as client boundary. All modules: {:?}",
        graph
            .modules
            .keys()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    );

    let provider = mock_provider.unwrap();
    assert!(provider.is_client);
    assert!(
        provider.exports.contains(&"MockProvider".to_string()),
        "Should detect MockProvider export, got: {:?}",
        provider.exports
    );
    assert!(
        provider.exports.contains(&"useMockContext".to_string()),
        "Should detect useMockContext export, got: {:?}",
        provider.exports
    );
    assert!(
        provider.exports.contains(&"default".to_string()),
        "Should detect default export, got: {:?}",
        provider.exports
    );
}

#[test]
fn node_modules_transitive_use_client_discovered() {
    let dir = setup_temp_dir();
    let root = dir.path();

    // Create a mock node_modules package that re-exports from a sub-module
    let pkg_dir = root.join("node_modules/mock-ui");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.json"),
        r#"{"name":"mock-ui","version":"1.0.0","main":"index.js"}"#,
    )
    .unwrap();
    // Main entry is "use client" and imports from a sub-module
    fs::write(
        pkg_dir.join("index.js"),
        r#"
"use client";
import { Button } from './components';
export { Button };
export function Provider({ children }) { return children; }
export default Provider;
"#,
    )
    .unwrap();
    // Sub-module is also "use client"
    fs::write(
        pkg_dir.join("components.js"),
        r#"
"use client";
export function Button() { return null; }
export function Input() { return null; }
"#,
    )
    .unwrap();

    // Server page imports from the package
    let app_dir = root.join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("page.tsx"),
        r#"
import { Provider } from 'mock-ui';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    // Both the main entry and the sub-module should be discovered
    let client_boundaries = graph.client_boundary_modules();
    let has_index = client_boundaries
        .iter()
        .any(|m| m.path.to_string_lossy().contains("mock-ui/index.js"));
    let has_components = client_boundaries
        .iter()
        .any(|m| m.path.to_string_lossy().contains("mock-ui/components.js"));

    assert!(has_index, "mock-ui/index.js should be a client boundary");
    assert!(
        has_components,
        "mock-ui/components.js should be discovered via shallow scan as a client boundary. \
         All modules: {:?}",
        graph
            .modules
            .keys()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn nested_providers_both_detected() {
    let dir = setup_temp_dir();
    let root = dir.path();

    let comp_dir = root.join("components");
    fs::create_dir_all(&comp_dir).unwrap();

    // ThemeProvider
    fs::write(
        comp_dir.join("ThemeProvider.tsx"),
        r#"
"use client";
export function ThemeProvider({ children }) { return children; }
export default ThemeProvider;
"#,
    )
    .unwrap();

    // AuthProvider
    fs::write(
        comp_dir.join("AuthProvider.tsx"),
        r#"
"use client";
export function AuthProvider({ children }) { return children; }
export function useAuth() { return {}; }
export default AuthProvider;
"#,
    )
    .unwrap();

    // Server page nests both providers
    let app_dir = root.join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("page.tsx"),
        r#"
import { ThemeProvider } from '../components/ThemeProvider';
import { AuthProvider } from '../components/AuthProvider';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let client_boundaries = graph.client_boundary_modules();
    assert_eq!(
        client_boundaries.len(),
        2,
        "Both ThemeProvider and AuthProvider should be client boundaries. Got: {:?}",
        client_boundaries
            .iter()
            .map(|m| m.path.display().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn provider_reexporting_from_another_client_module() {
    let dir = setup_temp_dir();
    let root = dir.path();

    let comp_dir = root.join("components");
    fs::create_dir_all(&comp_dir).unwrap();

    // Base context module
    fs::write(
        comp_dir.join("context.tsx"),
        r#"
"use client";
import { createContext, useContext } from 'react';
export const ThemeContext = createContext('light');
export function useTheme() { return useContext(ThemeContext); }
"#,
    )
    .unwrap();

    // Provider that re-exports from context module
    fs::write(
        comp_dir.join("Provider.tsx"),
        r#"
"use client";
import { ThemeContext } from './context';
export { useTheme } from './context';
export function ThemeProvider({ children }) { return children; }
export default ThemeProvider;
"#,
    )
    .unwrap();

    // Server page
    let app_dir = root.join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("page.tsx"),
        r#"
import { ThemeProvider } from '../components/Provider';
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let client_boundaries = graph.client_boundary_modules();

    // Both Provider and context should be client boundaries
    let has_provider = client_boundaries
        .iter()
        .any(|m| m.path.ends_with("Provider.tsx"));
    let has_context = client_boundaries
        .iter()
        .any(|m| m.path.ends_with("context.tsx"));

    assert!(has_provider, "Provider.tsx should be a client boundary");
    assert!(
        has_context,
        "context.tsx should be discovered as a client boundary via Provider's imports. \
         All modules: {:?}",
        graph
            .modules
            .keys()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn provider_with_dynamic_value_prop() {
    let dir = setup_temp_dir();
    let root = dir.path();

    let comp_dir = root.join("components");
    fs::create_dir_all(&comp_dir).unwrap();

    // Provider that accepts a dynamic value prop
    fs::write(
        comp_dir.join("ConfigProvider.tsx"),
        r#"
"use client";
import { createContext, useContext } from 'react';
export const ConfigContext = createContext({});
export function useConfig() { return useContext(ConfigContext); }
export function ConfigProvider({ config, children }) { return children; }
export default ConfigProvider;
"#,
    )
    .unwrap();

    // Server page passes a server-computed config value
    let app_dir = root.join("app");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("page.tsx"),
        r#"
import { ConfigProvider } from '../components/ConfigProvider';
const serverConfig = { apiUrl: 'https://api.example.com', debug: false };
export default function Page() { return null; }
"#,
    )
    .unwrap();

    let entries = vec![root.join("app/page.tsx")];
    let graph = analyze_module_graph(&entries, root).unwrap();

    let client_boundaries = graph.client_boundary_modules();
    let provider = client_boundaries
        .iter()
        .find(|m| m.path.ends_with("ConfigProvider.tsx"));

    assert!(
        provider.is_some(),
        "ConfigProvider with dynamic value prop should be a client boundary"
    );

    let p = provider.unwrap();
    assert!(p.exports.contains(&"ConfigContext".to_string()));
    assert!(p.exports.contains(&"useConfig".to_string()));
    assert!(p.exports.contains(&"ConfigProvider".to_string()));
    assert!(p.exports.contains(&"default".to_string()));
}
