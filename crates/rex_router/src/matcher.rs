use rex_core::{Route, RouteMatch};
use std::collections::HashMap;

/// A trie node for route matching
#[derive(Debug, Clone)]
struct TrieNode {
    /// Static children: segment string -> child node
    children: HashMap<String, TrieNode>,
    /// Dynamic parameter child: `:paramName`
    param_child: Option<(String, Box<TrieNode>)>,
    /// Catch-all child: `*paramName`
    catch_all: Option<(String, Route)>,
    /// Route at this node (if this is a terminal)
    route: Option<Route>,
}

impl TrieNode {
    fn new() -> Self {
        Self {
            children: HashMap::new(),
            param_child: None,
            catch_all: None,
            route: None,
        }
    }
}

/// Trie-based route matcher with priority: static > dynamic > catch-all
#[derive(Debug, Clone)]
pub struct RouteTrie {
    root: TrieNode,
}

impl RouteTrie {
    /// Build a trie from a list of routes
    pub fn from_routes(routes: &[Route]) -> Self {
        let mut trie = Self {
            root: TrieNode::new(),
        };
        for route in routes {
            trie.insert(route.clone());
        }
        trie
    }

    fn insert(&mut self, route: Route) {
        let pattern = route.pattern.clone();
        let segments = parse_segments(&pattern);
        let mut node = &mut self.root;

        for segment in &segments {
            if let Some(param_name) = segment.strip_prefix('*') {
                // Catch-all: store and stop
                node.catch_all = Some((param_name.to_string(), route));
                return;
            } else if let Some(param_name) = segment.strip_prefix(':') {
                // Dynamic parameter
                let (_, child) = node
                    .param_child
                    .get_or_insert_with(|| (param_name.to_string(), Box::new(TrieNode::new())));
                node = child.as_mut();
            } else {
                // Static segment
                node = node
                    .children
                    .entry(segment.to_string())
                    .or_insert_with(TrieNode::new);
            }
        }

        node.route = Some(route);
    }

    /// Collect all routes stored in the trie.
    pub fn routes(&self) -> Vec<&Route> {
        let mut result = Vec::new();
        Self::collect_routes(&self.root, &mut result);
        result
    }

    fn collect_routes<'a>(node: &'a TrieNode, out: &mut Vec<&'a Route>) {
        if let Some(route) = &node.route {
            out.push(route);
        }
        if let Some((_, route)) = &node.catch_all {
            out.push(route);
        }
        for child in node.children.values() {
            Self::collect_routes(child, out);
        }
        if let Some((_, child)) = &node.param_child {
            Self::collect_routes(child.as_ref(), out);
        }
    }

    /// Match a URL path against the trie. Returns the best match with extracted params.
    pub fn match_path(&self, path: &str) -> Option<RouteMatch> {
        let segments = parse_url_segments(path);
        let mut params = HashMap::new();
        self.match_node(&self.root, &segments, &mut params)
    }

    fn match_node(
        &self,
        node: &TrieNode,
        segments: &[&str],
        params: &mut HashMap<String, String>,
    ) -> Option<RouteMatch> {
        // Base case: no more segments to match
        if segments.is_empty() {
            if let Some(route) = &node.route {
                return Some(RouteMatch {
                    route: route.clone(),
                    params: params.clone(),
                });
            }
            // Check catch-all with empty match (optional catch-all)
            if let Some((name, route)) = &node.catch_all {
                let mut p = params.clone();
                p.insert(name.clone(), String::new());
                return Some(RouteMatch {
                    route: route.clone(),
                    params: p,
                });
            }
            return None;
        }

        let current = segments[0];
        let rest = &segments[1..];

        // Priority 1: Try static match
        if let Some(child) = node.children.get(current) {
            if let Some(m) = self.match_node(child, rest, params) {
                return Some(m);
            }
        }

        // Priority 2: Try dynamic parameter match
        if let Some((name, child)) = &node.param_child {
            params.insert(name.clone(), current.to_string());
            if let Some(m) = self.match_node(child.as_ref(), rest, params) {
                return Some(m);
            }
            params.remove(name);
        }

        // Priority 3: Try catch-all match
        if let Some((name, route)) = &node.catch_all {
            let mut p = params.clone();
            p.insert(name.clone(), segments.join("/"));
            return Some(RouteMatch {
                route: route.clone(),
                params: p,
            });
        }

        None
    }
}

fn parse_segments(pattern: &str) -> Vec<&str> {
    pattern
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_url_segments(path: &str) -> Vec<&str> {
    path.trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rex_core::{PageType, Route};
    use std::path::PathBuf;

    fn make_route(pattern: &str) -> Route {
        Route {
            pattern: pattern.to_string(),
            file_path: PathBuf::from(format!("{}.tsx", pattern.trim_start_matches('/'))),
            abs_path: PathBuf::from(format!("/pages/{}.tsx", pattern.trim_start_matches('/'))),
            dynamic_segments: vec![],
            page_type: PageType::Regular,
            specificity: 10,
        }
    }

    #[test]
    fn test_static_match() {
        let trie = RouteTrie::from_routes(&[make_route("/"), make_route("/about")]);

        let m = trie.match_path("/").unwrap();
        assert_eq!(m.route.pattern, "/");

        let m = trie.match_path("/about").unwrap();
        assert_eq!(m.route.pattern, "/about");

        assert!(trie.match_path("/nonexistent").is_none());
    }

    #[test]
    fn test_dynamic_match() {
        let trie = RouteTrie::from_routes(&[make_route("/blog/:slug")]);

        let m = trie.match_path("/blog/hello-world").unwrap();
        assert_eq!(m.route.pattern, "/blog/:slug");
        assert_eq!(m.params.get("slug").unwrap(), "hello-world");
    }

    #[test]
    fn test_static_over_dynamic() {
        let trie =
            RouteTrie::from_routes(&[make_route("/blog/featured"), make_route("/blog/:slug")]);

        let m = trie.match_path("/blog/featured").unwrap();
        assert_eq!(m.route.pattern, "/blog/featured");

        let m = trie.match_path("/blog/other").unwrap();
        assert_eq!(m.route.pattern, "/blog/:slug");
    }

    #[test]
    fn test_catch_all() {
        let trie = RouteTrie::from_routes(&[make_route("/docs/*path")]);

        let m = trie.match_path("/docs/a/b/c").unwrap();
        assert_eq!(m.params.get("path").unwrap(), "a/b/c");
    }

    #[test]
    fn test_catch_all_empty_match() {
        let trie = RouteTrie::from_routes(&[make_route("/docs/*path")]);
        let m = trie.match_path("/docs").unwrap();
        assert_eq!(m.params.get("path").unwrap(), "");
    }

    #[test]
    fn test_static_match_fails_falls_to_dynamic() {
        let trie =
            RouteTrie::from_routes(&[make_route("/blog/featured"), make_route("/blog/:slug")]);
        // Static match for "featured" should succeed
        let m = trie.match_path("/blog/featured").unwrap();
        assert_eq!(m.route.pattern, "/blog/featured");
        // Non-matching static falls through to dynamic
        let m = trie.match_path("/blog/other").unwrap();
        assert_eq!(m.route.pattern, "/blog/:slug");
        assert_eq!(m.params.get("slug").unwrap(), "other");
    }

    #[test]
    fn test_root_index() {
        let trie = RouteTrie::from_routes(&[make_route("/")]);
        let m = trie.match_path("/").unwrap();
        assert_eq!(m.route.pattern, "/");
    }

    #[test]
    fn test_routes_returns_all() {
        let routes = vec![
            make_route("/"),
            make_route("/about"),
            make_route("/blog/:slug"),
            make_route("/docs/*path"),
        ];
        let trie = RouteTrie::from_routes(&routes);
        let collected = trie.routes();
        assert_eq!(collected.len(), 4);
        let patterns: std::collections::HashSet<&str> =
            collected.iter().map(|r| r.pattern.as_str()).collect();
        assert!(patterns.contains("/"));
        assert!(patterns.contains("/about"));
        assert!(patterns.contains("/blog/:slug"));
        assert!(patterns.contains("/docs/*path"));
    }
}
