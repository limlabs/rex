/// Parse a space-delimited scope string into a set of individual scopes.
pub fn parse_scopes(scope_str: &str) -> Vec<String> {
    scope_str
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Check if `granted` scopes contain all `required` scopes.
pub fn has_scopes(granted: &[String], required: &[&str]) -> bool {
    required.iter().all(|r| granted.iter().any(|g| g == r))
}

/// Format scopes back into a space-delimited string.
pub fn format_scopes(scopes: &[String]) -> String {
    scopes.join(" ")
}

/// Validate that requested scopes are a subset of allowed scopes.
pub fn validate_scopes(requested: &[String], allowed: &[String]) -> Vec<String> {
    requested
        .iter()
        .filter(|s| allowed.iter().any(|a| a == *s))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_scopes() {
        let scopes = parse_scopes("tools:read tools:execute");
        assert_eq!(scopes, vec!["tools:read", "tools:execute"]);
    }

    #[test]
    fn test_parse_scopes_empty() {
        let scopes = parse_scopes("");
        assert!(scopes.is_empty());
    }

    #[test]
    fn test_parse_scopes_extra_spaces() {
        let scopes = parse_scopes("  a   b  c  ");
        assert_eq!(scopes, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_has_scopes() {
        let granted = vec!["tools:read".to_string(), "tools:execute".to_string()];
        assert!(has_scopes(&granted, &["tools:read"]));
        assert!(has_scopes(&granted, &["tools:read", "tools:execute"]));
        assert!(!has_scopes(&granted, &["admin"]));
    }

    #[test]
    fn test_format_scopes() {
        let scopes = vec!["a".to_string(), "b".to_string()];
        assert_eq!(format_scopes(&scopes), "a b");
    }

    #[test]
    fn test_validate_scopes() {
        let allowed = vec!["tools:read".to_string(), "tools:execute".to_string()];
        let requested = vec![
            "tools:read".to_string(),
            "admin".to_string(),
            "tools:execute".to_string(),
        ];
        let valid = validate_scopes(&requested, &allowed);
        assert_eq!(valid, vec!["tools:read", "tools:execute"]);
    }
}
