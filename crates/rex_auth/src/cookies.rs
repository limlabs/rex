use std::collections::HashMap;

/// Parse a `Cookie` header value into name-value pairs.
///
/// Handles the standard `name=value; name2=value2` format.
pub fn parse_cookies(cookie_header: &str) -> HashMap<String, String> {
    cookie_header
        .split(';')
        .filter_map(|pair| {
            let pair = pair.trim();
            let eq = pair.find('=')?;
            let name = pair[..eq].trim();
            let value = pair[eq + 1..].trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

/// Extract cookies from a `HashMap<String, String>` headers map (case-insensitive lookup for "cookie").
pub fn cookies_from_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    // Try both common casings
    if let Some(cookie_header) = headers.get("cookie").or_else(|| headers.get("Cookie")) {
        parse_cookies(cookie_header)
    } else {
        HashMap::new()
    }
}

/// Extract cookies from an Axum `HeaderMap`.
pub fn cookies_from_header_map(headers: &axum::http::HeaderMap) -> HashMap<String, String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .map(parse_cookies)
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_cookie() {
        let cookies = parse_cookies("name=value");
        assert_eq!(cookies.get("name").unwrap(), "value");
    }

    #[test]
    fn test_parse_multiple_cookies() {
        let cookies = parse_cookies("a=1; b=2; c=3");
        assert_eq!(cookies.len(), 3);
        assert_eq!(cookies.get("a").unwrap(), "1");
        assert_eq!(cookies.get("b").unwrap(), "2");
        assert_eq!(cookies.get("c").unwrap(), "3");
    }

    #[test]
    fn test_parse_cookies_with_spaces() {
        let cookies = parse_cookies("  name = value ;  other = stuff  ");
        assert_eq!(cookies.get("name").unwrap(), "value");
        assert_eq!(cookies.get("other").unwrap(), "stuff");
    }

    #[test]
    fn test_parse_empty_cookie() {
        let cookies = parse_cookies("");
        assert!(cookies.is_empty());
    }

    #[test]
    fn test_parse_cookie_with_equals_in_value() {
        let cookies = parse_cookies("token=abc=def==");
        assert_eq!(cookies.get("token").unwrap(), "abc=def==");
    }

    #[test]
    fn test_cookies_from_headers() {
        let mut headers = HashMap::new();
        headers.insert("cookie".to_string(), "session=abc123".to_string());
        let cookies = cookies_from_headers(&headers);
        assert_eq!(cookies.get("session").unwrap(), "abc123");
    }

    #[test]
    fn test_cookies_from_headers_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("Cookie".to_string(), "session=abc123".to_string());
        let cookies = cookies_from_headers(&headers);
        assert_eq!(cookies.get("session").unwrap(), "abc123");
    }

    #[test]
    fn test_cookies_from_headers_missing() {
        let headers = HashMap::new();
        let cookies = cookies_from_headers(&headers);
        assert!(cookies.is_empty());
    }
}
