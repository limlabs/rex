use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rex_build::AssetManifest;
use rex_server::document::{assemble_document, DocumentParams};
use rex_server::state::snapshot;
use rex_server::{Rex, RexOptions};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Convert a displayable error into a Python RuntimeError.
fn to_py_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(format!("{e:#}"))
}

/// Build a module_name -> route_pattern lookup from scan results.
fn build_module_map(scan: &rex_router::ScanResult) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for route in &scan.routes {
        map.insert(route.module_name(), route.pattern.clone());
    }
    map
}

/// Resolved component identity for rendering.
#[derive(Debug)]
struct ResolvedComponent {
    module_name: String,
    route_pattern: String,
}

/// Resolve a component string into module name + route pattern.
///
/// If `component` starts with `/`, it's treated as a URL path and matched
/// against the route trie. Otherwise, it's treated as a module name and
/// the pattern is looked up from the module map.
fn resolve_component(
    component: &str,
    rex: &Rex,
    module_to_pattern: &HashMap<String, String>,
) -> Result<ResolvedComponent, String> {
    if component.starts_with('/') {
        let route_match = rex
            .match_route(component)
            .ok_or_else(|| format!("No route matches path: {component}"))?;
        Ok(ResolvedComponent {
            module_name: route_match.module_name.clone(),
            route_pattern: route_match.pattern.clone(),
        })
    } else {
        let route_pattern = module_to_pattern
            .get(component)
            .cloned()
            .unwrap_or_else(|| format!("/{component}"));
        Ok(ResolvedComponent {
            module_name: component.to_string(),
            route_pattern,
        })
    }
}

/// Collect client JS scripts for a page from the manifest.
fn client_scripts_for(manifest: &AssetManifest, route_pattern: &str) -> Vec<String> {
    manifest
        .pages
        .get(route_pattern)
        .map(|p| vec![p.js.clone()])
        .unwrap_or_default()
}

/// Collect CSS files for a page (global + per-page) from the manifest.
fn css_files_for(manifest: &AssetManifest, route_pattern: &str) -> Vec<String> {
    manifest
        .pages
        .get(route_pattern)
        .map(|p| {
            let mut css = manifest.global_css.clone();
            css.extend(p.css.clone());
            css
        })
        .unwrap_or_else(|| manifest.global_css.clone())
}

/// A Rex renderer for Python web frameworks.
///
/// Embeds the Rex bundler (Rolldown) and SSR engine (V8) as a native extension.
/// No subprocess, no IPC, no separate binary.
///
/// Usage:
///     from rex_py import Renderer
///     rex = Renderer(root="./ui")
///     html = rex.render("index", props={"message": "Hello"})
#[pyclass]
struct Renderer {
    rex: Rex,
    /// Module name -> route pattern mapping for manifest lookups.
    module_to_pattern: HashMap<String, String>,
    /// Dedicated tokio runtime for async operations.
    rt: tokio::runtime::Runtime,
    closed: AtomicBool,
}

#[pymethods]
impl Renderer {
    #[new]
    #[pyo3(signature = (root="./ui", dev=false, pool_size=None))]
    fn new(root: &str, dev: bool, pool_size: Option<usize>) -> PyResult<Self> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .try_init();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(pool_size.unwrap_or(2).max(2))
            .enable_all()
            .build()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to create tokio runtime: {e}")))?;

        let rex = rt
            .block_on(Rex::new(RexOptions {
                root: root.into(),
                dev,
                port: 0,
                ..Default::default()
            }))
            .map_err(to_py_err)?;

        let module_to_pattern = build_module_map(rex.scan());

        Ok(Renderer {
            rex,
            module_to_pattern,
            rt,
            closed: AtomicBool::new(false),
        })
    }

    #[pyo3(signature = (component, props=None))]
    fn render(
        &self,
        py: Python<'_>,
        component: &str,
        props: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        self.check_closed()?;

        let props_json = match props {
            Some(obj) => python_to_json(obj)?,
            None => "{}".to_string(),
        };

        let resolved = resolve_component(component, &self.rex, &self.module_to_pattern)
            .map_err(|e| PyRuntimeError::new_err(e))?;

        let state = self.rex.state();

        // Release the GIL during V8 execution
        let result = py.detach(|| {
            let route_key = resolved.module_name.clone();
            let props = props_json.clone();
            self.rt.block_on(async {
                state
                    .isolate_pool
                    .execute(move |iso| iso.render_page(&route_key, &props))
                    .await
            })
        });

        let render_result = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(PyRuntimeError::new_err(format!("SSR render error: {e}"))),
            Err(e) => return Err(PyRuntimeError::new_err(format!("Pool error: {e}"))),
        };

        let hot = snapshot(&state);
        let client_scripts = client_scripts_for(&hot.manifest, &resolved.route_pattern);
        let css_files = css_files_for(&hot.manifest, &resolved.route_pattern);

        let html = assemble_document(&DocumentParams {
            ssr_html: &render_result.body,
            head_html: &render_result.head,
            props_json: &props_json,
            client_scripts: &client_scripts,
            css_files: &css_files,
            css_contents: &hot.manifest.css_contents,
            app_script: hot.manifest.app_script.as_deref(),
            is_dev: state.is_dev,
            doc_descriptor: hot.document_descriptor.as_ref(),
            manifest_json: Some(&hot.manifest_json),
            font_preloads: &hot.manifest.font_preloads,
            import_map_json: None,
        });

        Ok(html)
    }

    #[getter]
    fn client_dir(&self) -> String {
        self.rex.static_dir().to_string_lossy().to_string()
    }

    #[getter]
    fn build_id(&self) -> String {
        self.rex.build_id()
    }

    #[getter]
    fn manifest(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let hot = snapshot(&self.rex.state());
        let json_str = serde_json::to_string(&hot.manifest)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to serialize manifest: {e}")))?;
        let json_module = py.import("json")?;
        let result = json_module.call_method1("loads", (&json_str,))?;
        Ok(result.unbind())
    }

    #[getter]
    fn pages(&self) -> Vec<String> {
        self.module_to_pattern.keys().cloned().collect()
    }

    fn rebuild(&mut self) -> PyResult<()> {
        self.check_closed()?;

        let config = self.rex.config().clone();
        let project_config =
            rex_core::ProjectConfig::load(&config.project_root).map_err(to_py_err)?;
        let scan =
            rex_router::scan_project(&config.project_root, &config.pages_dir, &config.app_dir)
                .map_err(to_py_err)?;

        let build_result = self
            .rt
            .block_on(rex_build::build_bundles(&config, &scan, &project_config))
            .map_err(to_py_err)?;

        let server_bundle = std::fs::read_to_string(&build_result.server_bundle_path)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to read server bundle: {e}")))?;

        self.rt
            .block_on(
                self.rex
                    .state()
                    .isolate_pool
                    .reload_all(Arc::new(server_bundle)),
            )
            .map_err(to_py_err)?;

        self.module_to_pattern = build_module_map(&scan);

        Ok(())
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn __repr__(&self) -> String {
        let pages = self.module_to_pattern.len();
        let build_id = self.rex.build_id();
        format!("Renderer(pages={pages}, build_id=\"{build_id}\")")
    }
}

impl Renderer {
    fn check_closed(&self) -> PyResult<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(PyRuntimeError::new_err("Renderer is closed"));
        }
        Ok(())
    }
}

/// Serialize a Python object to a JSON string.
fn python_to_json(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    let py = obj.py();
    let json_module = py.import("json")?;
    let json_str: String = json_module.call_method1("dumps", (obj,))?.extract()?;
    Ok(json_str)
}

/// The rex_py Python module.
#[pymodule]
fn rex_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Renderer>()?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_build::AssetManifest;
    use rex_core::DataStrategy;
    use std::sync::LazyLock;

    /// Shared Rex instance — avoids concurrent builds racing on the same output dir.
    static REX: LazyLock<Rex> = LazyLock::new(|| {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("fixtures/basic");
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(Rex::new(RexOptions {
            root: fixtures,
            dev: false,
            port: 0,
            ..Default::default()
        }))
        .unwrap()
    });

    #[test]
    fn build_module_map_from_scan() {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("fixtures/basic");
        let pages_dir = fixtures.join("pages");
        let app_dir = fixtures.join("app");
        let scan = rex_router::scan_project(&fixtures, &pages_dir, &app_dir).unwrap();
        let map = build_module_map(&scan);

        assert!(map.contains_key("index"));
        assert!(map.contains_key("about"));
        assert!(map.contains_key("blog/[slug]"));
        assert_eq!(map.get("index").unwrap(), "/");
        assert_eq!(map.get("about").unwrap(), "/about");
        assert_eq!(map.get("blog/[slug]").unwrap(), "/blog/:slug");
    }

    #[test]
    fn resolve_component_by_module_name() {
        let mut module_to_pattern = HashMap::new();
        module_to_pattern.insert("index".to_string(), "/".to_string());
        module_to_pattern.insert("blog/[slug]".to_string(), "/blog/:slug".to_string());

        let rex = &*REX;

        // Known module name
        let resolved = resolve_component("index", rex, &module_to_pattern).unwrap();
        assert_eq!(resolved.module_name, "index");
        assert_eq!(resolved.route_pattern, "/");

        // Known dynamic module
        let resolved = resolve_component("blog/[slug]", rex, &module_to_pattern).unwrap();
        assert_eq!(resolved.module_name, "blog/[slug]");
        assert_eq!(resolved.route_pattern, "/blog/:slug");

        // Unknown module — falls back to /{name}
        let resolved = resolve_component("unknown", rex, &module_to_pattern).unwrap();
        assert_eq!(resolved.module_name, "unknown");
        assert_eq!(resolved.route_pattern, "/unknown");
    }

    #[test]
    fn resolve_component_by_url_path() {
        let module_to_pattern = HashMap::new();
        let rex = &*REX;

        // URL path matching
        let resolved = resolve_component("/about", rex, &module_to_pattern).unwrap();
        assert_eq!(resolved.module_name, "about");
        assert_eq!(resolved.route_pattern, "/about");

        // Dynamic URL path
        let resolved = resolve_component("/blog/hello", rex, &module_to_pattern).unwrap();
        assert_eq!(resolved.module_name, "blog/[slug]");
        assert_eq!(resolved.route_pattern, "/blog/:slug");
    }

    #[test]
    fn resolve_component_unknown_path_errors() {
        let module_to_pattern = HashMap::new();
        let rex = &*REX;
        let result = resolve_component("/nonexistent/path", rex, &module_to_pattern);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No route matches"));
    }

    #[test]
    fn client_scripts_for_known_page() {
        let mut manifest = AssetManifest::new("test123".to_string());
        manifest.add_page("/", "index-abc.js", DataStrategy::None, false);
        manifest.add_page("/about", "about-def.js", DataStrategy::None, false);

        assert_eq!(client_scripts_for(&manifest, "/"), vec!["index-abc.js"]);
        assert_eq!(
            client_scripts_for(&manifest, "/about"),
            vec!["about-def.js"]
        );
    }

    #[test]
    fn client_scripts_for_unknown_page() {
        let manifest = AssetManifest::new("test123".to_string());
        let scripts: Vec<String> = client_scripts_for(&manifest, "/unknown");
        assert!(scripts.is_empty());
    }

    #[test]
    fn css_files_includes_global_and_page() {
        let mut manifest = AssetManifest::new("test123".to_string());
        manifest.global_css = vec!["global.css".to_string()];
        manifest.add_page_with_css(
            "/",
            "index.js",
            &["page.css".to_string()],
            DataStrategy::None,
            false,
        );

        let css = css_files_for(&manifest, "/");
        assert_eq!(css, vec!["global.css", "page.css"]);
    }

    #[test]
    fn css_files_falls_back_to_global() {
        let mut manifest = AssetManifest::new("test123".to_string());
        manifest.global_css = vec!["global.css".to_string()];

        let css = css_files_for(&manifest, "/unknown");
        assert_eq!(css, vec!["global.css"]);
    }
}
