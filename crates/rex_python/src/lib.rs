use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rex_server::document::{assemble_document, DocumentParams};
use rex_server::handlers::snapshot;
use rex_server::{Rex, RexOptions};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Convert an anyhow::Error into a Python RuntimeError.
fn to_py_err(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(format!("{e:#}"))
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
    /// Create a new Renderer.
    ///
    /// Scans pages, bundles with Rolldown, and creates a V8 isolate pool.
    /// All in-process — no Node.js, no npm, no external binary.
    ///
    /// Args:
    ///     root: Directory containing pages/ (default: "./ui")
    ///     dev: Enable dev mode with file watching (default: False)
    ///     pool_size: Number of V8 isolates (default: CPU count)
    #[new]
    #[pyo3(signature = (root="./ui", dev=false, pool_size=None))]
    fn new(root: &str, dev: bool, pool_size: Option<usize>) -> PyResult<Self> {
        // Initialize tracing (ignore if already initialized)
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
                port: 0, // Python extension doesn't serve directly
            }))
            .map_err(to_py_err)?;

        // Build module_name -> route_pattern mapping from scan results
        let mut module_to_pattern = HashMap::new();
        for route in &rex.scan().routes {
            module_to_pattern.insert(route.module_name(), route.pattern.clone());
        }

        Ok(Renderer {
            rex,
            module_to_pattern,
            rt,
            closed: AtomicBool::new(false),
        })
    }

    /// Render a page component with the given props.
    ///
    /// Accepts either a module name ("blog/[slug]") or a URL path ("/blog/hello").
    /// Props are passed directly to the component — no getServerSideProps is called.
    /// Returns a complete HTML document string.
    ///
    /// The GIL is released during V8 execution, so multiple Python threads
    /// can call render() concurrently.
    ///
    /// Args:
    ///     component: Module name (e.g. "index", "blog/[slug]") or URL path
    ///     props: Dict of props to pass to the component (default: {})
    ///
    /// Returns:
    ///     Complete HTML document string
    #[pyo3(signature = (component, props=None))]
    fn render(
        &self,
        py: Python<'_>,
        component: &str,
        props: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        self.check_closed()?;

        // Serialize Python props to JSON
        let props_json = match props {
            Some(obj) => python_to_json(obj)?,
            None => "{}".to_string(),
        };

        // Determine the module name (route key for V8)
        // If it looks like a URL path (starts with /), match it against the trie.
        // Otherwise, treat it as a module name directly.
        let (module_name, route_pattern) = if component.starts_with('/') {
            let route_match = self.rex.match_route(component).ok_or_else(|| {
                PyRuntimeError::new_err(format!("No route matches path: {component}"))
            })?;
            let mod_name = route_match.module_name.clone();
            let pattern = route_match.pattern.clone();
            (mod_name, pattern)
        } else {
            let mod_name = component.to_string();
            let pattern = self
                .module_to_pattern
                .get(component)
                .cloned()
                .unwrap_or_else(|| format!("/{component}"));
            (mod_name, pattern)
        };

        let state = self.rex.state();

        // Release the GIL during V8 execution
        let result = py.detach(|| {
            let route_key = module_name.clone();
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

        // Look up page assets from the manifest
        let hot = snapshot(&state);
        let page_assets = hot.manifest.pages.get(&route_pattern);

        let client_scripts: Vec<String> =
            page_assets.map(|p| vec![p.js.clone()]).unwrap_or_default();

        let css_files: Vec<String> = page_assets
            .map(|p| {
                let mut css = hot.manifest.global_css.clone();
                css.extend(p.css.clone());
                css
            })
            .unwrap_or_else(|| hot.manifest.global_css.clone());

        // Assemble the full HTML document
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
        });

        Ok(html)
    }

    /// Path to the client-side static files directory.
    ///
    /// Mount this as a static file directory in your web framework:
    ///     app.mount("/_rex/static", StaticFiles(directory=rex.client_dir))
    #[getter]
    fn client_dir(&self) -> String {
        self.rex.static_dir().to_string_lossy().to_string()
    }

    /// The current build ID (content hash).
    #[getter]
    fn build_id(&self) -> String {
        self.rex.build_id()
    }

    /// Build manifest as a Python dict.
    #[getter]
    fn manifest(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let hot = snapshot(&self.rex.state());
        let json_str = serde_json::to_string(&hot.manifest)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to serialize manifest: {e}")))?;
        let json_module = py.import("json")?;
        let result = json_module.call_method1("loads", (&json_str,))?;
        Ok(result.unbind())
    }

    /// List of available page module names.
    #[getter]
    fn pages(&self) -> Vec<String> {
        self.module_to_pattern.keys().cloned().collect()
    }

    /// Rebuild bundles and reload V8 isolates.
    fn rebuild(&mut self) -> PyResult<()> {
        self.check_closed()?;

        let config = self.rex.config().clone();
        let project_config =
            rex_core::ProjectConfig::load(&config.project_root).map_err(to_py_err)?;
        let scan =
            rex_router::scan_project(&config.project_root, &config.pages_dir).map_err(to_py_err)?;

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

        // Update module mapping
        self.module_to_pattern.clear();
        for route in &scan.routes {
            self.module_to_pattern
                .insert(route.module_name(), route.pattern.clone());
        }

        Ok(())
    }

    /// Shut down the renderer, releasing V8 isolates and other resources.
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
