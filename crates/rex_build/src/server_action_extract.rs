//! Extract inline `"use server"` functions from server component source files.
//!
//! React 19 allows inline server actions in JSX:
//!   `<form action={async (fd) => { "use server"; ... }}>`
//!
//! Rex needs to hoist these to module level and register them with
//! `registerServerReference` so the RSC flight protocol can serialize them.
//! This module performs a source-to-source transform that extracts inline
//! server actions and replaces them with references to the hoisted functions.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// An extracted inline server action.
#[derive(Debug)]
pub struct ExtractedAction {
    /// Generated function name (e.g., `__rex_action_0`).
    pub name: String,
    /// The full extracted function source (hoisted to module level).
    pub source: String,
}

/// Result of extracting inline server actions from a source file.
#[derive(Debug)]
pub struct ExtractionResult {
    /// The transformed source with inline functions replaced by references.
    pub source: String,
    /// The extracted actions.
    pub actions: Vec<ExtractedAction>,
}

/// Extract inline `"use server"` functions from source code.
///
/// Returns `None` if no inline server actions were found.
pub fn extract_inline_server_actions(source: &str, _file_path: &Path) -> Option<ExtractionResult> {
    // Find all "use server" / 'use server' positions
    let positions = find_use_server_positions(source);
    if positions.is_empty() {
        return None;
    }

    // Filter out module-level directives (appear before any code)
    let first_code_pos = find_first_code_position(source);
    let inline_positions: Vec<usize> = positions
        .into_iter()
        .filter(|&pos| pos >= first_code_pos)
        .collect();

    if inline_positions.is_empty() {
        return None;
    }

    // For each inline "use server", find the enclosing function span
    let mut replacements: Vec<Replacement> = Vec::new();
    for (idx, &use_server_pos) in inline_positions.iter().enumerate() {
        if let Some(func_span) = find_enclosing_function(source, use_server_pos) {
            let action_name = format!("__rex_action_{idx}");
            let func_text = &source[func_span.start..func_span.end];

            // Build the hoisted function
            let hoisted = build_hoisted_function(func_text, &action_name);

            replacements.push(Replacement {
                start: func_span.start,
                end: func_span.end,
                action_name,
                hoisted_source: hoisted,
            });
        }
    }

    if replacements.is_empty() {
        return None;
    }

    // Sort replacements by position (descending) so we can apply them
    // without invalidating positions
    replacements.sort_by(|a, b| b.start.cmp(&a.start));

    // Build the transformed source
    let mut transformed = source.to_string();
    let mut actions = Vec::new();

    for rep in &replacements {
        // Replace the inline function with just the action name
        transformed.replace_range(rep.start..rep.end, &rep.action_name);

        actions.push(ExtractedAction {
            name: rep.action_name.clone(),
            source: rep.hoisted_source.clone(),
        });
    }

    // Prepend the hoisted functions at the top of the file (after imports)
    let insert_pos = find_insert_position(&transformed);
    let mut hoisted_block = String::new();
    // Reverse to maintain original order (we collected in reverse)
    for action in actions.iter().rev() {
        hoisted_block.push('\n');
        hoisted_block.push_str(&action.source);
        hoisted_block.push('\n');
    }

    transformed.insert_str(insert_pos, &hoisted_block);

    // Reverse actions to match original order
    actions.reverse();

    Some(ExtractionResult {
        source: transformed,
        actions,
    })
}

struct Replacement {
    start: usize,
    end: usize,
    action_name: String,
    hoisted_source: String,
}

/// Span of a function expression in the source.
struct FuncSpan {
    start: usize,
    end: usize,
}

/// Find all byte positions where `"use server"` or `'use server'` appears.
fn find_use_server_positions(source: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    for (i, _) in source.match_indices("\"use server\"") {
        positions.push(i);
    }
    for (i, _) in source.match_indices("'use server'") {
        positions.push(i);
    }
    positions.sort_unstable();
    positions
}

/// Find the byte position of the first non-import, non-directive code.
/// Module-level `"use server"` directives appear before this position.
fn find_first_code_position(source: &str) -> usize {
    let mut pos = 0;
    let bytes = source.as_bytes();
    let len = bytes.len();

    while pos < len {
        // Skip whitespace
        while pos < len
            && (bytes[pos] == b' '
                || bytes[pos] == b'\t'
                || bytes[pos] == b'\n'
                || bytes[pos] == b'\r')
        {
            pos += 1;
        }
        if pos >= len {
            break;
        }

        // Skip single-line comments
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }

        // Skip multi-line comments
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            pos += 2;
            while pos + 1 < len && !(bytes[pos] == b'*' && bytes[pos + 1] == b'/') {
                pos += 1;
            }
            pos += 2;
            continue;
        }

        // Check for string directives ("use server", "use client", "use strict")
        if bytes[pos] == b'"' || bytes[pos] == b'\'' {
            let quote = bytes[pos];
            let start = pos;
            pos += 1;
            while pos < len && bytes[pos] != quote {
                pos += 1;
            }
            if pos < len {
                pos += 1; // skip closing quote
            }
            // Skip optional semicolon
            while pos < len && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                pos += 1;
            }
            if pos < len && bytes[pos] == b';' {
                pos += 1;
            }
            // Check if this was a directive (starts with "use ")
            let content = &source[start..pos];
            if content.contains("use ") {
                continue;
            }
            // Not a directive — this is the first code
            return start;
        }

        // Check for import statements
        if source[pos..].starts_with("import ") || source[pos..].starts_with("import{") {
            // Skip the entire import statement
            while pos < len && bytes[pos] != b';' && bytes[pos] != b'\n' {
                // Handle multi-line imports
                if bytes[pos] == b'{' {
                    while pos < len && bytes[pos] != b'}' {
                        pos += 1;
                    }
                }
                pos += 1;
            }
            if pos < len {
                pos += 1;
            }
            continue;
        }

        // Any other statement — this is the first code position
        return pos;
    }

    len
}

/// Find the enclosing arrow function or function expression around a `"use server"` position.
///
/// Walks backwards from the directive to find the function start, then forward
/// to find the matching closing brace.
fn find_enclosing_function(source: &str, use_server_pos: usize) -> Option<FuncSpan> {
    let bytes = source.as_bytes();

    // Walk backwards from "use server" to find the opening `{` of the function body
    let mut pos = use_server_pos;
    while pos > 0 {
        pos -= 1;
        if bytes[pos] == b'{' {
            break;
        }
        // Skip whitespace and the directive itself
        if bytes[pos] == b' ' || bytes[pos] == b'\t' || bytes[pos] == b'\n' || bytes[pos] == b'\r' {
            continue;
        }
        // If we hit something that isn't whitespace or `{`, this "use server"
        // isn't directly after a function body opening
        return None;
    }

    if pos == 0 && bytes[0] != b'{' {
        return None;
    }

    let body_open_pos = pos; // position of `{`

    // Walk backwards from `{` to find `=>` (arrow function) or `function` keyword
    #[allow(clippy::almost_swapped)] // not a swap — saving pos then resetting
    {
        pos = body_open_pos;
    }
    while pos > 0 {
        pos -= 1;
        if bytes[pos] == b' ' || bytes[pos] == b'\t' || bytes[pos] == b'\n' || bytes[pos] == b'\r' {
            continue;
        }
        break;
    }

    // Check for arrow function: `=>`
    let is_arrow = pos > 0 && bytes[pos] == b'>' && bytes[pos - 1] == b'=';

    let func_start = if is_arrow {
        // Arrow function: walk back past `=>`, params, and optional `async`
        find_arrow_start(source, pos - 1)
    } else {
        // Regular function expression: walk back to find `function` keyword
        find_function_start(source, body_open_pos)
    }?;

    // Find the matching `}` for the opening `{`
    let body_end = find_matching_brace(source, body_open_pos)?;

    Some(FuncSpan {
        start: func_start,
        end: body_end + 1, // include the closing `}`
    })
}

/// Find the start of an arrow function by walking backwards from `=`.
/// Handles: `async (params) =>`, `(params) =>`, `async param =>`
fn find_arrow_start(source: &str, arrow_eq_pos: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut pos = arrow_eq_pos;

    // Skip backwards past whitespace
    while pos > 0 {
        pos -= 1;
        if bytes[pos] != b' ' && bytes[pos] != b'\t' && bytes[pos] != b'\n' && bytes[pos] != b'\r' {
            break;
        }
    }

    let params_end = pos;

    // If we're at `)`, find matching `(`
    if bytes[params_end] == b')' {
        let mut depth = 1;
        pos = params_end;
        while pos > 0 && depth > 0 {
            pos -= 1;
            match bytes[pos] {
                b')' => depth += 1,
                b'(' => depth -= 1,
                _ => {}
            }
        }
        // pos is now at `(`
    } else {
        // Single param without parens: walk back to find the start of the identifier
        while pos > 0
            && (bytes[pos - 1].is_ascii_alphanumeric()
                || bytes[pos - 1] == b'_'
                || bytes[pos - 1] == b'$')
        {
            pos -= 1;
        }
    }

    let params_start = pos;

    // Check for `async` keyword before params
    let mut check_pos = params_start;
    while check_pos > 0
        && (bytes[check_pos - 1] == b' '
            || bytes[check_pos - 1] == b'\t'
            || bytes[check_pos - 1] == b'\n'
            || bytes[check_pos - 1] == b'\r')
    {
        check_pos -= 1;
    }

    // Check if the word before params is "async"
    if check_pos >= 5 && &source[check_pos - 5..check_pos] == "async" {
        // Verify 'async' isn't part of a larger identifier
        if check_pos < 6
            || !bytes[check_pos - 6].is_ascii_alphanumeric() && bytes[check_pos - 6] != b'_'
        {
            return Some(check_pos - 5);
        }
    }

    Some(params_start)
}

/// Find the start of a `function` expression by walking backwards.
fn find_function_start(source: &str, body_open_pos: usize) -> Option<usize> {
    // Walk backwards from `{` looking for the `function` keyword
    let search_region = &source[..body_open_pos];
    let func_pos = search_region.rfind("function")?;

    // Check for `async` before `function`
    let before_func = &source[..func_pos];
    let trimmed = before_func.trim_end();
    if trimmed.ends_with("async") {
        let async_pos = trimmed.len() - 5;
        return Some(async_pos);
    }

    Some(func_pos)
}

/// Find the matching closing `}` for an opening `{` at `open_pos`.
fn find_matching_brace(source: &str, open_pos: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = b' ';
    let mut in_template = false;
    let mut pos = open_pos;

    while pos < bytes.len() {
        let ch = bytes[pos];

        if in_string {
            if ch == string_char && (pos == 0 || bytes[pos - 1] != b'\\') {
                in_string = false;
            }
            pos += 1;
            continue;
        }

        if in_template {
            if ch == b'`' && (pos == 0 || bytes[pos - 1] != b'\\') {
                in_template = false;
            } else if ch == b'$' && pos + 1 < bytes.len() && bytes[pos + 1] == b'{' {
                // Template expression — track braces
                depth += 1;
                pos += 2;
                continue;
            }
            pos += 1;
            continue;
        }

        match ch {
            b'"' | b'\'' => {
                in_string = true;
                string_char = ch;
            }
            b'`' => {
                in_template = true;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            // Skip line comments
            b'/' if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' => {
                pos += 2;
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            // Skip block comments
            b'/' if pos + 1 < bytes.len() && bytes[pos + 1] == b'*' => {
                pos += 2;
                while pos + 1 < bytes.len() && !(bytes[pos] == b'*' && bytes[pos + 1] == b'/') {
                    pos += 1;
                }
                pos += 2;
                continue;
            }
            _ => {}
        }
        pos += 1;
    }
    None
}

/// Build a hoisted function from the inline function text.
///
/// Converts `async (params) => { "use server"; body }` to
/// `async function __rex_action_0(params) { body }`
fn build_hoisted_function(func_text: &str, action_name: &str) -> String {
    let is_async = func_text.trim_start().starts_with("async");

    // Find the params and body
    let arrow_pos = func_text.find("=>").unwrap_or(0);
    let body_open = func_text[arrow_pos..].find('{').map(|p| p + arrow_pos);

    if let Some(body_start) = body_open {
        // Extract params (between first `(` and matching `)`)
        let params_start = func_text.find('(');
        let params_end = func_text[..arrow_pos].rfind(')');

        let params = match (params_start, params_end) {
            (Some(s), Some(e)) => &func_text[s..=e],
            _ => "()",
        };

        // Extract body content (between `{` and matching `}`)
        let body_content = &func_text[body_start + 1..func_text.len() - 1];

        // Remove the "use server" directive from body
        let cleaned_body = remove_use_server_directive(body_content);

        let async_prefix = if is_async { "async " } else { "" };
        format!("export {async_prefix}function {action_name}{params} {{{cleaned_body}}}")
    } else {
        // Couldn't parse — just wrap it
        format!("export const {action_name} = {func_text};")
    }
}

/// Remove `"use server"` or `'use server'` directive from function body text.
fn remove_use_server_directive(body: &str) -> String {
    let mut result = body.to_string();

    // Remove "use server" with optional semicolon and surrounding whitespace
    for pattern in &["\"use server\"", "'use server'"] {
        if let Some(pos) = result.find(pattern) {
            let mut end = pos + pattern.len();
            let bytes = result.as_bytes();
            // Skip optional whitespace and semicolon after directive
            while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b';' {
                end += 1;
            }
            // Skip trailing newline
            if end < bytes.len() && bytes[end] == b'\n' {
                end += 1;
            } else if end + 1 < bytes.len() && bytes[end] == b'\r' && bytes[end + 1] == b'\n' {
                end += 2;
            }
            result.replace_range(pos..end, "");
        }
    }

    result
}

/// Find the position to insert hoisted functions (after all imports).
fn find_insert_position(source: &str) -> usize {
    let mut last_import_end = 0;
    let mut pos = 0;
    let bytes = source.as_bytes();
    let len = bytes.len();

    while pos < len {
        // Skip whitespace
        while pos < len
            && (bytes[pos] == b' '
                || bytes[pos] == b'\t'
                || bytes[pos] == b'\n'
                || bytes[pos] == b'\r')
        {
            pos += 1;
        }
        if pos >= len {
            break;
        }

        // Skip comments
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'/' {
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if pos + 1 < len && bytes[pos] == b'/' && bytes[pos + 1] == b'*' {
            pos += 2;
            while pos + 1 < len && !(bytes[pos] == b'*' && bytes[pos + 1] == b'/') {
                pos += 1;
            }
            pos += 2;
            continue;
        }

        // Skip string directives
        if bytes[pos] == b'"' || bytes[pos] == b'\'' {
            let quote = bytes[pos];
            pos += 1;
            while pos < len && bytes[pos] != quote {
                pos += 1;
            }
            if pos < len {
                pos += 1;
            }
            while pos < len && (bytes[pos] == b' ' || bytes[pos] == b'\t' || bytes[pos] == b';') {
                pos += 1;
            }
            last_import_end = pos;
            continue;
        }

        // Check for import statement
        if source[pos..].starts_with("import ")
            || source[pos..].starts_with("import{")
            || source[pos..].starts_with("import*")
        {
            // Find end of import statement (semicolon or newline after from clause)
            while pos < len {
                if bytes[pos] == b';' {
                    pos += 1;
                    break;
                }
                if bytes[pos] == b'\n' && source[..pos].ends_with(['\'', '"']) {
                    pos += 1;
                    break;
                }
                pos += 1;
            }
            last_import_end = pos;
            continue;
        }

        // Non-import statement — stop
        break;
    }

    last_import_end
}

/// Rolldown plugin that intercepts file loads for modules with inline
/// `"use server"` directives and returns the transformed source with
/// actions hoisted to module level. Injects `registerServerReference`
/// calls inline so the registration happens in the same IIFE scope
/// as the function (critical for grouped route builds).
#[derive(Debug)]
pub struct InlineServerActionPlugin {
    /// Canonical paths of modules that need transformation.
    targets: HashSet<PathBuf>,
    /// Project root for computing relative paths.
    project_root: PathBuf,
    /// Build ID for action ID computation.
    build_id: String,
}

impl InlineServerActionPlugin {
    pub fn new(targets: Vec<PathBuf>, project_root: PathBuf, build_id: String) -> Self {
        Self {
            targets: targets.into_iter().collect(),
            project_root,
            build_id,
        }
    }
}

impl rolldown::plugin::Plugin for InlineServerActionPlugin {
    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("rex:inline-server-action")
    }

    fn load(
        &self,
        _ctx: rolldown::plugin::SharedLoadPluginContext,
        args: &rolldown::plugin::HookLoadArgs<'_>,
    ) -> impl std::future::Future<Output = rolldown::plugin::HookLoadReturn> + Send {
        let result = if self.targets.contains(Path::new(args.id)) {
            let source = std::fs::read_to_string(args.id).unwrap_or_default();
            let path = Path::new(args.id);
            match extract_inline_server_actions(&source, path) {
                Some(mut extraction) => {
                    // Inline registerServerReference: directly set the properties
                    // React checks for. Avoids importing react-server-dom-webpack
                    // which would break in group IIFEs without react-server condition.
                    let rel_path = path
                        .strip_prefix(&self.project_root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    let mut reg = String::new();
                    for action in &extraction.actions {
                        let action_id = crate::server_action_manifest::server_action_id(
                            &rel_path,
                            &action.name,
                            &self.build_id,
                        );
                        reg.push_str(&format!(
                            "{n}.$$typeof = Symbol.for(\"react.server.reference\");\
                            {n}.$$id = \"{id}\";\
                            {n}.$$bound = null;\n",
                            n = action.name,
                            id = action_id,
                        ));
                    }
                    extraction.source.push_str(&reg);
                    Some(rolldown::plugin::HookLoadOutput {
                        code: extraction.source.into(),
                        ..Default::default()
                    })
                }
                None => None,
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
#[allow(clippy::unwrap_used)]
#[path = "server_action_extract_tests.rs"]
mod tests;
