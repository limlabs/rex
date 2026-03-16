use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// How many visual lines `text` occupies when wrapped to `max_width`.
pub(super) fn wrapped_line_count(text: &str, max_width: usize) -> usize {
    if max_width == 0 {
        return 1;
    }
    let len = text.len();
    if len <= max_width {
        return 1;
    }
    len.div_ceil(max_width)
}

/// Wrap `text` into segments of at most `max_width` chars, preferring breaks
/// at spaces, slashes, colons, and dots so file paths and URLs stay readable.
pub(super) fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.len() <= max_width {
        return vec![text.to_string()];
    }

    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();

    while start < text.len() {
        let remaining = text.len() - start;
        if remaining <= max_width {
            lines.push(text[start..].to_string());
            break;
        }

        let end = start + max_width;
        // Look for a break character within the last third of the chunk
        let search_start = start + max_width * 2 / 3;
        let mut break_at = None;
        for i in (search_start..end).rev() {
            if i < bytes.len() && matches!(bytes[i], b' ' | b'/' | b'\\' | b':' | b'.' | b',') {
                break_at = Some(i + 1);
                break;
            }
        }
        let split = break_at.unwrap_or(end);
        lines.push(text[start..split].to_string());
        start = split;
    }

    if lines.is_empty() {
        lines.push(text.to_string());
    }
    lines
}

/// Push search-highlighted spans for `text` into `spans`.
pub(super) fn highlight_search(spans: &mut Vec<Span<'_>>, text: &str, query: &str) {
    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    let mut last_end = 0;
    let mut pos = 0;

    while let Some(idx) = text_lower[pos..].find(&query_lower) {
        let start = pos + idx;
        let end = start + query.len();
        if start > last_end {
            spans.push(Span::raw(text[last_end..start].to_string()));
        }
        spans.push(Span::styled(
            text[start..end].to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        last_end = end;
        pos = end;
    }
    if last_end < text.len() {
        spans.push(Span::raw(text[last_end..].to_string()));
    }
}

/// Split text by newlines, then wrap each resulting line to `max_width`.
/// Returns all visual lines.
pub(super) fn wrap_message(text: &str, max_width: usize) -> Vec<String> {
    let mut result = Vec::new();
    for line in text.lines() {
        result.extend(wrap_text(line, max_width));
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

/// Truncate `s` to at most `max_width` bytes, respecting UTF-8 char boundaries.
pub(super) fn truncate_str(s: &str, max_width: usize) -> &str {
    if s.len() <= max_width {
        return s;
    }
    let mut end = max_width;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_short_text_unchanged() {
        assert_eq!(wrap_text("hello", 80), vec!["hello"]);
    }

    #[test]
    fn wrap_at_space() {
        let result = wrap_text("hello world this is long", 12);
        assert_eq!(result, vec!["hello world ", "this is long"]);
    }

    #[test]
    fn wrap_at_slash() {
        let result = wrap_text("/usr/local/bin/very-long-filename", 20);
        // Should break at a slash within the last third
        assert!(result.len() >= 2);
        assert!(result[0].len() <= 20);
    }

    #[test]
    fn wrap_hard_break_no_separator() {
        let result = wrap_text("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(result, vec!["abcdefghij", "klmnopqrst", "uvwxyz"]);
    }

    #[test]
    fn wrapped_line_count_short() {
        assert_eq!(wrapped_line_count("hello", 80), 1);
    }

    #[test]
    fn wrapped_line_count_long() {
        assert_eq!(wrapped_line_count("a]b]c]d]e]f", 4), 3);
    }

    #[test]
    fn wrapped_line_count_exact() {
        assert_eq!(wrapped_line_count("abcd", 4), 1);
    }

    #[test]
    fn wrapped_line_count_zero_width() {
        assert_eq!(wrapped_line_count("hello", 0), 1);
    }

    #[test]
    fn highlight_no_match() {
        let mut spans = Vec::new();
        highlight_search(&mut spans, "hello world", "xyz");
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn highlight_match_found() {
        let mut spans = Vec::new();
        highlight_search(&mut spans, "hello world", "world");
        assert!(spans.len() >= 2); // "hello " + highlighted "world"
    }

    #[test]
    fn highlight_case_insensitive() {
        let mut spans = Vec::new();
        highlight_search(&mut spans, "Hello World", "hello");
        assert!(!spans.is_empty());
    }

    #[test]
    fn wrap_message_splits_newlines() {
        let result = wrap_message("line1\nline2\nline3", 80);
        assert_eq!(result, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn wrap_message_newline_and_wrap() {
        let result = wrap_message("short\nabcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(result[0], "short");
        assert!(result.len() >= 3);
    }

    #[test]
    fn truncate_str_within_limit() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn truncate_str_over_limit() {
        assert_eq!(truncate_str("hello world", 5), "hello");
    }
}
