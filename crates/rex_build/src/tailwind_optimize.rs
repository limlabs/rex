use anyhow::Result;
use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};

/// Optimize CSS output with optional minification.
///
/// Uses lightningcss (Rust-native) to minify CSS (whitespace removal,
/// shorthand merging) when `minify` is true. No vendor prefixing is
/// performed — modern browsers don't need it for Tailwind's output.
pub fn optimize_css(css: &str, minify: bool) -> Result<String> {
    let mut stylesheet = StyleSheet::parse(css, ParserOptions::default())
        .map_err(|e| anyhow::anyhow!("lightningcss parse error: {e}"))?;

    if minify {
        stylesheet
            .minify(MinifyOptions::default())
            .map_err(|e| anyhow::anyhow!("lightningcss minify error: {e}"))?;
    }

    let result = stylesheet
        .to_css(PrinterOptions {
            minify,
            ..Default::default()
        })
        .map_err(|e| anyhow::anyhow!("lightningcss print error: {e}"))?;

    Ok(result.code)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_optimize_passthrough() {
        let css = "body { margin: 0; padding: 0; }";
        let result = optimize_css(css, false).unwrap();
        assert!(result.contains("margin"));
        assert!(result.contains("padding"));
    }

    #[test]
    fn test_optimize_minify() {
        let css = "body {\n  margin: 0;\n  padding: 0;\n}\n";
        let result = optimize_css(css, true).unwrap();
        // Minified output should be shorter (no unnecessary whitespace)
        assert!(result.len() < css.len());
        assert!(result.contains("margin"));
    }

    #[test]
    fn test_optimize_empty() {
        let result = optimize_css("", false).unwrap();
        assert!(result.is_empty() || result.trim().is_empty());
    }
}
