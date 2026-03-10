use anyhow::Result;
use tracing::debug;

/// Embedded Tailwind CSS compiler, bundled at cargo build time.
/// Self-contained CJS wrapped in an IIFE that exposes `__tw_compile` and
/// `__tw_css` (the embedded base stylesheets) on globalThis.
const TAILWIND_COMPILER_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/tailwind-compiler.js"));

/// V8 polyfills (console, TextEncoder, performance, etc.) required by the
/// Tailwind compiler in bare V8 (no Node.js runtime).
use crate::server_bundle::V8_POLYFILLS;

/// The JS snippet evaluated after the compiler is loaded. It defines the
/// `loadStylesheet` resolver and a `__tw_run(css, candidates)` helper that
/// calls `compile()` + `build()`.
const TAILWIND_SETUP_JS: &str = r#"
globalThis.__tw_run = async function(inputCss, candidatesJson) {
    function loadStylesheet(id) {
        var css = globalThis.__tw_css;
        if (id === 'tailwindcss' || id === 'tailwindcss/index.css')
            return { content: css.index, base: 'tailwindcss' };
        if (id === 'tailwindcss/preflight' || id === 'tailwindcss/preflight.css' || id === './preflight.css')
            return { content: css.preflight, base: 'tailwindcss' };
        if (id === 'tailwindcss/theme' || id === 'tailwindcss/theme.css' || id === './theme.css')
            return { content: css.theme, base: 'tailwindcss' };
        if (id === 'tailwindcss/utilities' || id === 'tailwindcss/utilities.css' || id === './utilities.css')
            return { content: css.utilities, base: 'tailwindcss' };
        throw new Error('Cannot resolve Tailwind stylesheet: ' + id);
    }
    var candidates = JSON.parse(candidatesJson);
    var compiler = await globalThis.__tw_compile(inputCss, {
        loadStylesheet: loadStylesheet,
        base: '/',
    });
    return compiler.build(candidates);
};
"#;

/// Compile Tailwind CSS using the embedded V8-based compiler.
///
/// Creates a one-off V8 isolate, loads the pre-bundled Tailwind compiler,
/// passes the input CSS and candidate class names, and returns the compiled
/// CSS output.
pub fn compile_tailwind_v8(input_css: &str, candidates: &[String]) -> Result<String> {
    debug!(
        candidates = candidates.len(),
        "Compiling Tailwind CSS in V8"
    );

    // Encode candidates as JSON to pass into V8
    let candidates_json = serde_json::to_string(candidates)?;

    // Build the invocation script that passes the CSS and candidates
    let invoke_script = format!(
        "globalThis.__tw_run({input_css}, {candidates_json})",
        input_css = serde_json::to_string(input_css)?,
        candidates_json = serde_json::to_string(&candidates_json)?,
    );

    let css = rex_v8::eval_once(
        V8_POLYFILLS,
        &[
            (TAILWIND_COMPILER_JS, "<tailwind-compiler>"),
            (TAILWIND_SETUP_JS, "<tailwind-setup>"),
            (&invoke_script, "<tailwind-compile>"),
        ],
    )?;

    debug!(
        output_bytes = css.len(),
        "Tailwind CSS compilation complete"
    );
    Ok(css)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_basic_utilities() {
        let css = compile_tailwind_v8(
            r#"@import "tailwindcss";"#,
            &[
                "bg-red-500".to_string(),
                "p-4".to_string(),
                "text-white".to_string(),
            ],
        )
        .unwrap();

        assert!(!css.is_empty(), "compiled CSS should not be empty");
        // Should contain the utility classes
        assert!(
            css.contains("bg-red-500") || css.contains("background-color"),
            "should contain bg-red-500 utility"
        );
        assert!(
            css.contains("p-4") || css.contains("padding"),
            "should contain p-4 utility"
        );
    }

    #[test]
    fn test_compile_empty_candidates() {
        let css = compile_tailwind_v8(r#"@import "tailwindcss";"#, &[]).unwrap();
        // Even with no candidates, Tailwind produces base/preflight CSS
        assert!(!css.is_empty());
    }

    #[test]
    fn test_compile_responsive_variants() {
        let css = compile_tailwind_v8(
            r#"@import "tailwindcss";"#,
            &["sm:text-lg".to_string(), "md:flex".to_string()],
        )
        .unwrap();
        assert!(!css.is_empty());
    }
}
