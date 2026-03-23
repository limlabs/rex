//! Integration tests for RSC bundle builder.
//! Extracted from rsc_bundler.rs inline tests.

#[allow(clippy::unwrap_used)]
mod tests {
    use rex_build::rsc_bundler::build_rsc_bundles;
    use rex_core::app_route::{AppRoute, AppScanResult, AppSegment};
    use std::fs;
    use std::path::Path;

    /// Assert that an IIFE bundle has no unresolved external parameters.
    ///
    /// Rolldown wraps IIFE bundles as `(function(ext) { ... })(ext)` when it treats
    /// an import as external. The trailing `)(ext);` will fail at V8 eval time because
    /// `ext` is not a global. This check catches missing resolve aliases at build time.
    fn assert_no_iife_externals(bundle_js: &str, label: &str) {
        // The IIFE footer pattern: `})(some_identifier);` at the end of the bundle.
        // A self-contained IIFE ends with `})();\n` (no arguments).
        let trimmed = bundle_js.trim_end();
        if let Some(tail) = trimmed.strip_suffix(';') {
            if let Some(before_paren) = tail.strip_suffix(')') {
                // Find the matching '(' for the IIFE call arguments
                if let Some(args_start) = before_paren.rfind(")(") {
                    let args = &before_paren[args_start + 2..];
                    assert!(
                        args.is_empty(),
                        "{label} has unresolved IIFE externals: `({args})` — \
                         this means rolldown treated an import as external. \
                         Add a resolve alias for the missing module."
                    );
                }
            }
        }
    }

    /// Symlink the real node_modules from fixtures/app-router into a test directory.
    fn setup_rsc_node_modules(root: &Path) {
        let fixture_nm =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/app-router/node_modules");
        assert!(
            fixture_nm.exists(),
            "fixtures/app-router/node_modules not found — run `cd fixtures/app-router && npm install`"
        );
        std::os::unix::fs::symlink(
            fixture_nm.canonicalize().unwrap(),
            root.join("node_modules"),
        )
        .unwrap();
        // package.json is needed so the build doesn't try to use built-in modules
        fs::write(
            root.join("package.json"),
            r#"{"name":"rsc-test","private":true}"#,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_rsc_build_produces_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_node_modules(&root);

        // Create app directory with layout + page
        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "export default function Home() { return 'Hello'; }\n",
        )
        .unwrap();

        // Create a "use client" component
        let comp_dir = root.join("components");
        fs::create_dir_all(&comp_dir).unwrap();
        let counter_path = comp_dir.join("Counter.tsx");
        fs::write(
            &counter_path,
            "\"use client\";\nexport default function Counter() { return 'count'; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
                route: None,
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![AppRoute {
                pattern: "/".to_string(),
                page_path: page_path.clone(),
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![None],
                error_chain: vec![None],
                dynamic_segments: vec![],
                specificity: 10,
                route_group: None,
            }],
            api_routes: vec![],
            root_layout: Some(layout_path),
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-build-id", &define, false, None)
            .await
            .expect("build_rsc_bundles should succeed");

        // Server bundle file exists
        let server_path = result
            .server_bundle_path
            .as_ref()
            .expect("server bundle path");
        assert!(
            server_path.exists(),
            "Server bundle should exist at {:?}",
            server_path
        );

        // Server bundle is non-empty
        let server_content = fs::read_to_string(server_path).unwrap();
        assert!(
            !server_content.is_empty(),
            "Server bundle should not be empty"
        );

        // SSR bundle file exists
        let ssr_path = result.ssr_bundle_path.as_ref().expect("ssr bundle path");
        assert!(
            ssr_path.exists(),
            "SSR bundle should exist at {:?}",
            ssr_path
        );

        // SSR bundle is non-empty
        let ssr_content = fs::read_to_string(ssr_path).unwrap();
        assert!(!ssr_content.is_empty(), "SSR bundle should not be empty");

        // Client manifest was created (may be empty if no "use client" modules in entries)
        // Verify the manifest struct exists and is accessible
        let _ = &result.client_manifest.entries;

        // Verify bundle has no unresolved externals in IIFE wrapper.
        // Unresolved imports produce `(function(some_external) { ... })(some_external);`
        // which fails at runtime since `some_external` is undefined.
        assert_no_iife_externals(&server_content, "RSC server bundle");
        assert_no_iife_externals(&ssr_content, "RSC SSR bundle");
    }

    #[tokio::test]
    async fn test_rsc_build_with_server_actions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_node_modules(&root);

        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        // Page that imports from a "use server" module
        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "import { increment } from './actions';\nexport default function Home() { return 'Hello'; }\n",
        )
        .unwrap();

        // "use server" module
        let actions_path = app_dir.join("actions.ts");
        fs::write(
            &actions_path,
            "\"use server\";\nexport async function increment(n: number) { return n + 1; }\nexport async function decrement(n: number) { return n - 1; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
                route: None,
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![AppRoute {
                pattern: "/".to_string(),
                page_path: page_path.clone(),
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![None],
                error_chain: vec![None],
                dynamic_segments: vec![],
                specificity: 10,
                route_group: None,
            }],
            api_routes: vec![],
            root_layout: Some(layout_path),
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-sa-build", &define, false, None)
            .await
            .expect("build_rsc_bundles should succeed");

        // Server action manifest should have 2 actions
        assert_eq!(
            result.server_action_manifest.actions.len(),
            2,
            "Should have 2 server actions (increment + decrement)"
        );

        // Verify actions are in the manifest
        let has_increment = result
            .server_action_manifest
            .actions
            .values()
            .any(|a| a.export_name == "increment");
        assert!(has_increment, "Manifest should contain increment action");

        let has_decrement = result
            .server_action_manifest
            .actions
            .values()
            .any(|a| a.export_name == "decrement");
        assert!(has_decrement, "Manifest should contain decrement action");

        // Server bundle should contain server action dispatch code
        let server_content =
            fs::read_to_string(result.server_bundle_path.as_ref().unwrap()).unwrap();
        assert!(
            server_content.contains("__rex_server_actions"),
            "Server bundle should contain action dispatch table"
        );
        assert!(
            server_content.contains("__rex_call_server_action"),
            "Server bundle should contain action call function"
        );

        // Verify bundle has no unresolved externals in IIFE wrapper
        assert_no_iife_externals(&server_content, "RSC server bundle");
    }

    #[tokio::test]
    async fn test_client_bundle_uses_stubs_for_server_actions_imported_by_client_component() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_node_modules(&root);

        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        // Server page imports a client component (not the actions directly)
        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "import ActionCounter from '../components/ActionCounter';\nexport default function Home() { return 'Hello'; }\n",
        )
        .unwrap();

        // "use client" component imports from a "use server" module
        let comp_dir = root.join("components");
        fs::create_dir_all(&comp_dir).unwrap();
        fs::write(
            comp_dir.join("ActionCounter.tsx"),
            "\"use client\";\nimport { increment } from '../app/actions';\nexport default function ActionCounter() { return 'count: ' + increment(0); }\n",
        )
        .unwrap();

        // "use server" module
        fs::write(
            app_dir.join("actions.ts"),
            "\"use server\";\nexport async function increment(n: number) { return n + 1; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
                route: None,
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![AppRoute {
                pattern: "/".to_string(),
                page_path: page_path.clone(),
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![None],
                error_chain: vec![None],
                dynamic_segments: vec![],
                specificity: 10,
                route_group: None,
            }],
            api_routes: vec![],
            root_layout: Some(layout_path),
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-sa-client", &define, false, None)
            .await
            .expect("build_rsc_bundles should succeed");

        // Server action manifest should have the increment action
        assert_eq!(
            result.server_action_manifest.actions.len(),
            1,
            "Should have 1 server action (increment)"
        );

        // Find the client bundle for ActionCounter
        let client_dir = root.join(".rex/build/client/rsc");
        let action_counter_chunk = fs::read_dir(&client_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().contains("ActionCounter"))
            .expect("ActionCounter client chunk should exist");

        let client_content = fs::read_to_string(action_counter_chunk.path()).unwrap();

        // The client bundle must use createServerReference, NOT inline the function body
        assert!(
            client_content.contains("createServerReference"),
            "Client bundle should use createServerReference proxy, not inline the function. Got: {client_content}"
        );
        assert!(
            !client_content.contains("return n + 1"),
            "Client bundle should NOT contain the server action implementation. Got: {client_content}"
        );
    }

    #[tokio::test]
    async fn test_rsc_build_no_client_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();

        setup_rsc_node_modules(&root);

        let app_dir = root.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let layout_path = app_dir.join("layout.tsx");
        fs::write(
            &layout_path,
            "export default function RootLayout({ children }) { return children; }\n",
        )
        .unwrap();

        let page_path = app_dir.join("page.tsx");
        fs::write(
            &page_path,
            "export default function Home() { return 'Hello server only'; }\n",
        )
        .unwrap();

        let config = rex_core::RexConfig::new(root.clone()).with_dev(true);

        let app_scan = AppScanResult {
            root: AppSegment {
                segment: String::new(),
                layout: Some(layout_path.clone()),
                page: Some(page_path.clone()),
                route: None,
                loading: None,
                error_boundary: None,
                not_found: None,
                children: vec![],
            },
            routes: vec![AppRoute {
                pattern: "/".to_string(),
                page_path: page_path.clone(),
                layout_chain: vec![layout_path.clone()],
                loading_chain: vec![None],
                error_chain: vec![None],
                dynamic_segments: vec![],
                specificity: 10,
                route_group: None,
            }],
            api_routes: vec![],
            root_layout: Some(layout_path),
        };

        let define = vec![(
            "process.env.NODE_ENV".to_string(),
            "\"development\"".to_string(),
        )];

        let result = build_rsc_bundles(&config, &app_scan, "test-no-client", &define, false, None)
            .await
            .expect("build_rsc_bundles should succeed");

        // No client boundaries → empty manifest entries (only placeholder entries if any)
        assert!(
            result.client_manifest.entries.is_empty(),
            "Manifest should be empty with no client boundaries"
        );

        // Server action manifest should be empty
        assert!(
            result.server_action_manifest.actions.is_empty(),
            "No server actions expected"
        );
    }
}
