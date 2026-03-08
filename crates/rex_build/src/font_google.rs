use anyhow::Result;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

/// Google Fonts CSS2 API endpoint.
const GOOGLE_FONTS_CSS_URL: &str = "https://fonts.googleapis.com/css2";

/// User agent to request woff2 format from Google Fonts.
const WOFF2_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36";

/// A font that has been fully processed (downloaded, CSS generated).
#[derive(Debug, Clone)]
pub(crate) struct ProcessedFont {
    pub scoped_family: String,
    pub css: String,
    pub preload_files: Vec<String>,
}

/// Process a single font: download from Google Fonts, generate CSS.
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_single_font(
    family: &str,
    weights: &[String],
    display: &str,
    variable: Option<&str>,
    fallback: &[String],
    output_dir: &Path,
    build_id: &str,
    cache_dir: &Path,
    processed: &mut HashMap<String, ProcessedFont>,
) -> Result<ProcessedFont> {
    let cache_key = format!("{}-{}", family, weights.join(","));

    if let Some(existing) = processed.get(&cache_key) {
        return Ok(existing.clone());
    }

    let hash = &short_hash(cache_key.as_bytes());
    let scoped_family = format!("__font_{}_{}", family.replace(' ', "_"), hash);

    // Try to download from Google Fonts
    let (font_face_css, preload_files) = match fetch_and_process_google_font(
        family,
        weights,
        display,
        output_dir,
        build_id,
        cache_dir,
        &scoped_family,
    ) {
        Ok(result) => result,
        Err(e) => {
            warn!(
                font = %family,
                error = %e,
                "Failed to download font, falling back to Google Fonts CDN"
            );
            let css = generate_cdn_fallback_css(family, weights, display, &scoped_family);
            (css, Vec::new())
        }
    };

    // Generate utility class CSS
    let mut css = font_face_css;

    let fallback_stack = if fallback.is_empty() {
        default_fallback(family)
    } else {
        fallback.join(", ")
    };
    css.push_str(&format!(
        ".{scoped_family} {{ font-family: '{scoped_family}', {fallback_stack}; }}\n"
    ));

    if let Some(var_name) = variable {
        css.push_str(&format!(
            ":root {{ {var_name}: '{scoped_family}', {fallback_stack}; }}\n"
        ));
    }

    let font = ProcessedFont {
        scoped_family,
        css,
        preload_files,
    };

    processed.insert(cache_key, font.clone());
    Ok(font)
}

/// Build a JS object literal from font config and scoped name.
pub(crate) fn build_font_object(
    family: &str,
    fallback: &[String],
    variable: Option<&str>,
    scoped_family: &str,
) -> String {
    let fallback_stack = if fallback.is_empty() {
        default_fallback(family)
    } else {
        fallback.join("', '")
    };

    let mut obj = format!(
        "{{ className: \"{scoped_family}\", style: {{ fontFamily: \"'{scoped_family}', {fallback_stack}\" }}"
    );

    if let Some(var) = variable {
        obj.push_str(&format!(", variable: \"{var}\""));
    }

    obj.push_str(" }");
    obj
}

/// Get the default fallback font stack for a font family.
pub(crate) fn default_fallback(family: &str) -> String {
    let lower = family.to_lowercase();
    if lower.contains("serif") && !lower.contains("sans") {
        "serif".to_string()
    } else if lower.contains("mono") || lower.contains("code") {
        "monospace".to_string()
    } else {
        "sans-serif".to_string()
    }
}

/// Fetch Google Font CSS, download woff2 files, and generate local @font-face rules.
fn fetch_and_process_google_font(
    family: &str,
    weights: &[String],
    display: &str,
    output_dir: &Path,
    build_id: &str,
    cache_dir: &Path,
    scoped_family: &str,
) -> Result<(String, Vec<String>)> {
    let hash = &build_id[..8.min(build_id.len())];

    let family_param = build_google_fonts_param(family, weights);
    let url = format!("{GOOGLE_FONTS_CSS_URL}?family={family_param}&display={display}");

    debug!(url = %url, "Fetching Google Font CSS");

    // Check cache first
    let css_cache_key = short_hash(url.as_bytes());
    let cached_css_path = cache_dir.join(format!("{css_cache_key}.css"));

    let css_text = if cached_css_path.exists() {
        debug!("Using cached Google Font CSS");
        fs::read_to_string(&cached_css_path)?
    } else {
        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(&url)
            .header("User-Agent", WOFF2_USER_AGENT)
            .send()?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Google Fonts API returned {}: {}",
                resp.status(),
                resp.text().unwrap_or_default()
            );
        }

        let text = resp.text()?;
        fs::write(&cached_css_path, &text)?;
        text
    };

    // Parse @font-face rules and download font files
    let font_faces = parse_google_font_css(&css_text);
    let mut local_css = String::new();
    let mut preload_files = Vec::new();
    let mut downloaded_urls: HashMap<String, String> = HashMap::new();

    for face in &font_faces {
        let filename = if let Some(cached) = downloaded_urls.get(&face.src_url) {
            cached.clone()
        } else {
            let font_filename = download_font_file(
                &face.src_url,
                family,
                &face.weight,
                &face.style,
                hash,
                output_dir,
                cache_dir,
            )?;
            downloaded_urls.insert(face.src_url.clone(), font_filename.clone());
            font_filename
        };

        if face.is_latin {
            preload_files.push(filename.clone());
        }

        local_css.push_str("@font-face {\n");
        local_css.push_str(&format!("  font-family: '{scoped_family}';\n"));
        local_css.push_str(&format!(
            "  src: url('/_rex/static/{filename}') format('woff2');\n"
        ));
        local_css.push_str(&format!("  font-weight: {};\n", face.weight));
        local_css.push_str(&format!("  font-style: {};\n", face.style));
        local_css.push_str(&format!("  font-display: {display};\n"));
        if !face.unicode_range.is_empty() {
            local_css.push_str(&format!("  unicode-range: {};\n", face.unicode_range));
        }
        local_css.push_str("}\n");
    }

    Ok((local_css, preload_files))
}

/// Build the `family` parameter for Google Fonts CSS2 API.
pub(crate) fn build_google_fonts_param(family: &str, weights: &[String]) -> String {
    let family = family.replace(' ', "+");
    if weights.len() == 1 && weights[0] == "variable" {
        format!("{family}:wght@100..900")
    } else {
        let weights = weights.join(";");
        format!("{family}:wght@{weights}")
    }
}

/// Parsed @font-face rule from Google Fonts CSS.
#[derive(Debug)]
struct GoogleFontFace {
    src_url: String,
    weight: String,
    style: String,
    unicode_range: String,
    is_latin: bool,
}

/// Parse Google Fonts CSS response to extract @font-face information.
fn parse_google_font_css(css: &str) -> Vec<GoogleFontFace> {
    let mut faces = Vec::new();
    let mut i = 0;
    let bytes = css.as_bytes();
    let mut current_subset = String::new();

    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            if let Some(end) = css[i + 2..].find("*/") {
                let comment = css[i + 2..i + 2 + end].trim();
                current_subset = comment.to_string();
                i += end + 4;
                continue;
            }
        }

        if css[i..].starts_with("@font-face") {
            if let Some(brace_start) = css[i..].find('{') {
                let block_start = i + brace_start + 1;
                if let Some(brace_end) = css[block_start..].find('}') {
                    let block = &css[block_start..block_start + brace_end];

                    let src_url = extract_css_url(block).unwrap_or_default();
                    let weight = extract_css_property(block, "font-weight")
                        .unwrap_or_else(|| "400".to_string());
                    let style = extract_css_property(block, "font-style")
                        .unwrap_or_else(|| "normal".to_string());
                    let unicode_range =
                        extract_css_property(block, "unicode-range").unwrap_or_default();

                    if !src_url.is_empty() {
                        faces.push(GoogleFontFace {
                            src_url,
                            weight,
                            style,
                            unicode_range,
                            is_latin: current_subset == "latin",
                        });
                    }

                    i = block_start + brace_end + 1;
                    continue;
                }
            }
        }

        i += 1;
    }

    faces
}

/// Extract URL from a `src:` CSS property value.
fn extract_css_url(block: &str) -> Option<String> {
    let src_start = block.find("src:")?;
    let after_src = &block[src_start + 4..];
    let url_start = after_src.find("url(")?;
    let url_content_start = url_start + 4;
    let url_end = after_src[url_content_start..].find(')')?;
    let url = after_src[url_content_start..url_content_start + url_end].trim();
    Some(url.trim_matches(|c| c == '\'' || c == '"').to_string())
}

/// Extract a CSS property value from a declaration block.
fn extract_css_property(block: &str, property: &str) -> Option<String> {
    let prop_pattern = format!("{property}:");
    let start = block.find(&prop_pattern)?;
    let after = &block[start + prop_pattern.len()..];
    let end = after.find(';').unwrap_or(after.len());
    Some(after[..end].trim().to_string())
}

/// Download a font file and save to output directory.
fn download_font_file(
    url: &str,
    family: &str,
    weight: &str,
    style: &str,
    hash: &str,
    output_dir: &Path,
    cache_dir: &Path,
) -> Result<String> {
    let url_hash = short_hash(url.as_bytes());
    let safe_family = family.to_lowercase().replace(' ', "-");
    let filename = format!("{safe_family}-{weight}-{style}-{url_hash}-{hash}.woff2");

    let output_path = output_dir.join(&filename);
    if output_path.exists() {
        return Ok(filename);
    }

    let cache_path = cache_dir.join(format!("{url_hash}.woff2"));
    if cache_path.exists() {
        fs::copy(&cache_path, &output_path)?;
        debug!(file = %filename, "Font file copied from cache");
        return Ok(filename);
    }

    debug!(url = %url, file = %filename, "Downloading font file");
    let client = reqwest::blocking::Client::new();
    let resp = client.get(url).send()?;

    if !resp.status().is_success() {
        anyhow::bail!("Failed to download font file: {}", resp.status());
    }

    let bytes = resp.bytes()?;
    fs::write(&cache_path, &bytes)?;
    fs::write(&output_path, &bytes)?;

    Ok(filename)
}

/// Generate fallback @font-face CSS pointing to Google Fonts CDN.
fn generate_cdn_fallback_css(
    family: &str,
    weights: &[String],
    display: &str,
    scoped_family: &str,
) -> String {
    let family_param = build_google_fonts_param(family, weights);
    let import_url = format!("{GOOGLE_FONTS_CSS_URL}?family={family_param}&display={display}");

    let mut css = format!("@import url('{import_url}');\n");

    for weight in weights {
        css.push_str("@font-face {\n");
        css.push_str(&format!("  font-family: '{scoped_family}';\n"));
        css.push_str(&format!("  src: local('{family}');\n"));
        css.push_str(&format!("  font-weight: {weight};\n"));
        css.push_str(&format!("  font-display: {display};\n"));
        css.push_str("}\n");
    }

    css
}

/// Compute a short hex hash.
pub(crate) fn short_hash(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    hex::encode(&hasher.finalize()[..4])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_build_google_fonts_param_single_weight() {
        assert_eq!(
            build_google_fonts_param("Inter", &["400".to_string()]),
            "Inter:wght@400"
        );
    }

    #[test]
    fn test_build_google_fonts_param_multiple_weights() {
        assert_eq!(
            build_google_fonts_param("Roboto", &["400".to_string(), "700".to_string()]),
            "Roboto:wght@400;700"
        );
    }

    #[test]
    fn test_build_google_fonts_param_variable() {
        assert_eq!(
            build_google_fonts_param("Inter", &["variable".to_string()]),
            "Inter:wght@100..900"
        );
    }

    #[test]
    fn test_build_google_fonts_param_spaces() {
        assert_eq!(
            build_google_fonts_param("Open Sans", &["400".to_string()]),
            "Open+Sans:wght@400"
        );
    }

    #[test]
    fn test_parse_google_font_css() {
        let css = r#"/* latin-ext */
@font-face {
  font-family: 'Inter';
  font-style: normal;
  font-weight: 400;
  font-display: swap;
  src: url(https://fonts.gstatic.com/s/inter/v18/abc.woff2) format('woff2');
  unicode-range: U+0100-02BA;
}
/* latin */
@font-face {
  font-family: 'Inter';
  font-style: normal;
  font-weight: 400;
  font-display: swap;
  src: url(https://fonts.gstatic.com/s/inter/v18/def.woff2) format('woff2');
  unicode-range: U+0000-00FF;
}
"#;
        let faces = parse_google_font_css(css);
        assert_eq!(faces.len(), 2);
        assert_eq!(
            faces[0].src_url,
            "https://fonts.gstatic.com/s/inter/v18/abc.woff2"
        );
        assert!(!faces[0].is_latin);
        assert!(faces[1].is_latin);
        assert_eq!(faces[0].weight, "400");
        assert_eq!(faces[0].style, "normal");
    }

    #[test]
    fn test_extract_css_url() {
        let block = "  src: url(https://example.com/font.woff2) format('woff2');";
        assert_eq!(
            extract_css_url(block),
            Some("https://example.com/font.woff2".to_string())
        );
    }

    #[test]
    fn test_extract_css_property() {
        let block = "  font-weight: 700;\n  font-style: italic;\n  font-display: swap;\n";
        assert_eq!(
            extract_css_property(block, "font-weight"),
            Some("700".to_string())
        );
        assert_eq!(
            extract_css_property(block, "font-style"),
            Some("italic".to_string())
        );
    }

    #[test]
    fn test_short_hash_deterministic() {
        assert_eq!(short_hash(b"hello"), short_hash(b"hello"));
    }

    #[test]
    fn test_short_hash_differs() {
        assert_ne!(short_hash(b"hello"), short_hash(b"world"));
    }

    #[test]
    fn test_default_fallback() {
        assert_eq!(default_fallback("Inter"), "sans-serif");
        assert_eq!(default_fallback("Roboto Mono"), "monospace");
        assert_eq!(default_fallback("Source Code Pro"), "monospace");
        assert_eq!(default_fallback("Noto Serif"), "serif");
    }

    #[test]
    fn test_build_font_object_basic() {
        let result = build_font_object("Inter", &[], None, "__font_Inter_abc123");
        assert!(result.contains("className: \"__font_Inter_abc123\""));
        assert!(result.contains("fontFamily:"));
        assert!(result.contains("sans-serif"));
        assert!(!result.contains("variable"));
    }

    #[test]
    fn test_build_font_object_with_variable() {
        let result = build_font_object(
            "Roboto",
            &["system-ui".to_string()],
            Some("--font-roboto"),
            "__font_Roboto_def456",
        );
        assert!(result.contains("className: \"__font_Roboto_def456\""));
        assert!(result.contains("variable: \"--font-roboto\""));
    }

    #[test]
    fn test_generate_cdn_fallback_css_single_weight() {
        let css = generate_cdn_fallback_css("Inter", &["400".to_string()], "swap", "__font_Inter");
        assert!(css.contains("@import url('"));
        assert!(css.contains("Inter:wght@400"));
        assert!(css.contains("font-family: '__font_Inter'"));
        assert!(css.contains("font-weight: 400"));
        assert!(css.contains("font-display: swap"));
        assert!(css.contains("src: local('Inter')"));
    }

    #[test]
    fn test_generate_cdn_fallback_css_multiple_weights() {
        let css = generate_cdn_fallback_css(
            "Roboto",
            &["400".to_string(), "700".to_string()],
            "block",
            "__font_Roboto",
        );
        // Should have one @import and two @font-face blocks
        assert_eq!(css.matches("@import").count(), 1);
        assert_eq!(css.matches("@font-face").count(), 2);
        assert!(css.contains("font-weight: 400"));
        assert!(css.contains("font-weight: 700"));
        assert!(css.contains("font-display: block"));
    }

    #[test]
    fn test_parse_google_font_css_no_faces() {
        let faces = parse_google_font_css("/* nothing here */\nbody { color: red; }");
        assert!(faces.is_empty());
    }

    #[test]
    fn test_parse_google_font_css_missing_url() {
        let css = "@font-face { font-weight: 400; font-style: normal; }";
        let faces = parse_google_font_css(css);
        assert!(faces.is_empty()); // no src url → skipped
    }

    #[test]
    fn test_parse_google_font_css_defaults() {
        // No font-weight or font-style in the block
        let css = r#"/* latin */
@font-face {
  src: url(https://example.com/f.woff2) format('woff2');
}
"#;
        let faces = parse_google_font_css(css);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].weight, "400"); // default
        assert_eq!(faces[0].style, "normal"); // default
        assert!(faces[0].unicode_range.is_empty());
        assert!(faces[0].is_latin);
    }

    #[test]
    fn test_extract_css_url_quoted() {
        let block = "  src: url('https://example.com/font.woff2') format('woff2');";
        assert_eq!(
            extract_css_url(block),
            Some("https://example.com/font.woff2".to_string())
        );
    }

    #[test]
    fn test_extract_css_url_none() {
        assert_eq!(extract_css_url("font-weight: 400;"), None);
    }

    #[test]
    fn test_extract_css_property_missing() {
        let block = "font-weight: 400;";
        assert_eq!(extract_css_property(block, "font-style"), None);
    }

    #[test]
    fn test_process_single_font_caches_result() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        let output_dir = tmp.path().join("out");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&output_dir).unwrap();

        let mut processed: HashMap<String, ProcessedFont> = HashMap::new();

        // Insert a pre-processed font to test the cache hit path
        let cached = ProcessedFont {
            scoped_family: "__font_Inter_cached".to_string(),
            css: ".cached {}".to_string(),
            preload_files: vec!["cached.woff2".to_string()],
        };
        processed.insert("Inter-400".to_string(), cached.clone());

        let result = process_single_font(
            "Inter",
            &["400".to_string()],
            "swap",
            None,
            &[],
            &output_dir,
            "build123",
            &cache_dir,
            &mut processed,
        )
        .unwrap();

        assert_eq!(result.scoped_family, "__font_Inter_cached");
        assert_eq!(result.css, ".cached {}");
    }

    #[test]
    fn test_process_single_font_utility_class_css() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        let output_dir = tmp.path().join("out");
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&output_dir).unwrap();

        let mut processed: HashMap<String, ProcessedFont> = HashMap::new();

        // This will fail to fetch from Google Fonts (no network in tests),
        // which triggers the CDN fallback path — testing that code path.
        let result = process_single_font(
            "Inter",
            &["400".to_string()],
            "swap",
            Some("--font-inter"),
            &["system-ui".to_string()],
            &output_dir,
            "build12345678",
            &cache_dir,
            &mut processed,
        )
        .unwrap();

        // Should have generated utility class CSS
        assert!(result.css.contains(&format!(".{}", result.scoped_family)));
        assert!(result.css.contains("font-family:"));
        assert!(result.css.contains("system-ui"));
        // Should have CSS variable rule
        assert!(result.css.contains("--font-inter"));
        assert!(result.css.contains(":root"));
        // Should be cached in the processed map
        assert!(processed.contains_key("Inter-400"));
    }
}
