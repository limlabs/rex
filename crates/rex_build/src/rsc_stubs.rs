//! Stub generators for RSC `"use client"` and `"use server"` module boundaries.
//!
//! Client reference stubs replace `"use client"` modules in the server flight bundle
//! with lightweight objects containing `$$typeof: Symbol.for("react.client.reference")`.
//!
//! Server action stubs replace `"use server"` modules in the client bundle with
//! `createServerReference` proxies that call back to the server.

use crate::client_manifest::client_reference_id;
use crate::server_action_manifest::server_action_id;

/// Generate a client reference stub module for a `"use client"` component.
///
/// For each export, produces:
/// ```js
/// export const Foo = { $$typeof: Symbol.for("react.client.reference"), $$id: "<refId>", $$name: "Foo" };
/// ```
pub(crate) fn generate_client_stub(rel_path: &str, exports: &[String], build_id: &str) -> String {
    let mut source = String::new();
    source.push_str("// Auto-generated client reference stub\n");

    for export in exports {
        let ref_id = client_reference_id(rel_path, export, build_id);
        let obj = format!(
            "{{ $$typeof: Symbol.for(\"react.client.reference\"), $$id: \"{ref_id}\", $$name: \"{export}\" }}"
        );

        if export == "default" {
            source.push_str(&format!("export default {obj};\n"));
        } else {
            source.push_str(&format!("export const {export} = {obj};\n"));
        }
    }

    source
}

/// Generate a server action stub module for a `"use server"` module in the client bundle.
///
/// For each export, produces:
/// ```js
/// import { createServerReference } from 'react-server-dom-webpack/client';
/// export const increment = createServerReference("actionId", window.__REX_CALL_SERVER);
/// ```
pub(crate) fn generate_server_action_stub(
    rel_path: &str,
    exports: &[String],
    build_id: &str,
) -> String {
    let mut source = String::new();
    source.push_str("// Auto-generated server action stub\n");
    source.push_str("import { createServerReference } from 'react-server-dom-webpack/client';\n");

    for export in exports {
        let action_id = server_action_id(rel_path, export, build_id);
        if export == "default" {
            source.push_str(&format!(
                "export default createServerReference(\"{action_id}\", window.__REX_CALL_SERVER);\n"
            ));
        } else {
            source.push_str(&format!(
                "export var {export} = createServerReference(\"{action_id}\", window.__REX_CALL_SERVER);\n"
            ));
        }
    }

    source
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn client_stub_default_export() {
        let stub = generate_client_stub("components/Counter.tsx", &["default".to_string()], "abc");
        assert!(stub.contains("export default"));
        assert!(stub.contains("react.client.reference"));
        assert!(stub.contains("$$name: \"default\""));
    }

    #[test]
    fn client_stub_named_exports() {
        let stub = generate_client_stub(
            "utils.tsx",
            &["Counter".to_string(), "Input".to_string()],
            "abc",
        );
        assert!(stub.contains("export const Counter"));
        assert!(stub.contains("export const Input"));
        assert!(!stub.contains("export default"));
    }

    #[test]
    fn client_stub_mixed_exports() {
        let stub = generate_client_stub(
            "comp.tsx",
            &["default".to_string(), "Helper".to_string()],
            "abc",
        );
        assert!(stub.contains("export default"));
        assert!(stub.contains("export const Helper"));
    }

    #[test]
    fn client_stub_empty_exports() {
        let stub = generate_client_stub("empty.tsx", &[], "abc");
        assert_eq!(stub, "// Auto-generated client reference stub\n");
    }

    #[test]
    fn server_action_stub_named_exports() {
        let stub = generate_server_action_stub(
            "app/actions.ts",
            &["increment".to_string(), "decrement".to_string()],
            "abc",
        );
        assert!(stub.contains("import { createServerReference }"));
        assert!(stub.contains("export var increment = createServerReference("));
        assert!(stub.contains("export var decrement = createServerReference("));
        assert!(stub.contains("window.__REX_CALL_SERVER"));
    }

    #[test]
    fn server_action_stub_default_export() {
        let stub = generate_server_action_stub("app/actions.ts", &["default".to_string()], "abc");
        assert!(stub.contains("export default createServerReference("));
    }

    #[test]
    fn server_action_stub_empty_exports() {
        let stub = generate_server_action_stub("empty.ts", &[], "abc");
        assert!(stub.contains("import { createServerReference }"));
        // Only header + import, no export lines
        assert!(!stub.contains("export"));
    }

    #[test]
    fn server_action_stub_import_appears_once() {
        let stub = generate_server_action_stub(
            "actions.ts",
            &["a".to_string(), "b".to_string(), "c".to_string()],
            "abc",
        );
        let count = stub.matches("import { createServerReference }").count();
        assert_eq!(count, 1, "import should appear exactly once");
    }
}
