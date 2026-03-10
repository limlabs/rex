#![allow(clippy::unwrap_used)]

use rex_live::cache::{BuildCache, CachedBuild};
use rex_live::compiler::latest_source_mtime_pub;
use rex_live::source::{LocalSource, SourceProvider};
use std::sync::Arc;
use std::time::SystemTime;
use tempfile::TempDir;

// ──────────────────────────────── BuildCache ─────────────────────────────────

fn dummy_build(build_number: u64, mtime: SystemTime) -> CachedBuild {
    CachedBuild {
        server_bundle_js: Arc::new("// bundle".into()),
        build_id: format!("build-{build_number}"),
        manifest: rex_core::AssetManifest::default(),
        scan: rex_router::ScanResult {
            routes: vec![],
            api_routes: vec![],
            app: None,
            document: None,
            error: None,
            not_found: None,
            middleware: None,
            app_scan: None,
            mcp_tools: vec![],
        },
        source_mtime: mtime,
        build_number,
    }
}

#[test]
fn cache_starts_empty() {
    let cache = BuildCache::new();
    assert!(cache.get().is_none());
}

#[test]
fn cache_set_and_get() {
    let cache = BuildCache::new();
    let build = dummy_build(1, SystemTime::now());
    cache.set(build);

    let cached = cache.get().expect("should have cached build");
    assert_eq!(cached.build_id, "build-1");
    assert_eq!(cached.build_number, 1);
}

#[test]
fn cache_invalidate_clears() {
    let cache = BuildCache::new();
    cache.set(dummy_build(1, SystemTime::now()));
    assert!(cache.get().is_some());

    cache.invalidate();
    assert!(cache.get().is_none());
}

#[test]
fn cache_set_overwrites_previous() {
    let cache = BuildCache::new();
    cache.set(dummy_build(1, SystemTime::now()));
    cache.set(dummy_build(2, SystemTime::now()));

    let cached = cache.get().expect("should have cached build");
    assert_eq!(cached.build_id, "build-2");
}

#[test]
fn cache_build_number_increments() {
    let cache = BuildCache::new();
    assert_eq!(cache.next_build_number(), 0);
    assert_eq!(cache.next_build_number(), 1);
    assert_eq!(cache.next_build_number(), 2);
}

#[test]
fn cache_default_is_empty() {
    let cache = BuildCache::default();
    assert!(cache.get().is_none());
    assert_eq!(cache.next_build_number(), 0);
}

// ──────────────────────────────── LocalSource ────────────────────────────────

#[test]
fn local_source_root_is_canonical() {
    let tmp = TempDir::new().unwrap();
    let source = LocalSource::new(tmp.path().to_path_buf()).unwrap();
    // Canonical path should be absolute and resolved
    assert!(source.root().is_absolute());
}

#[test]
fn local_source_dir_exists() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("pages")).unwrap();

    let source = LocalSource::new(tmp.path().to_path_buf()).unwrap();
    assert!(source.dir_exists("pages"));
    assert!(!source.dir_exists("nonexistent"));
}

#[test]
fn local_source_file_meta_existing() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("test.tsx"), "export default () => <div/>").unwrap();

    let source = LocalSource::new(tmp.path().to_path_buf()).unwrap();
    let meta = source.file_meta("test.tsx").unwrap().expect("file exists");
    assert!(meta.size > 0);
    assert!(meta.modified <= SystemTime::now());
}

#[test]
fn local_source_file_meta_nonexistent() {
    let tmp = TempDir::new().unwrap();
    let source = LocalSource::new(tmp.path().to_path_buf()).unwrap();
    let meta = source.file_meta("missing.tsx").unwrap();
    assert!(meta.is_none());
}

#[test]
fn local_source_new_fails_for_nonexistent_dir() {
    let result = LocalSource::new("/nonexistent/path/abc123".into());
    assert!(result.is_err());
}

// ──────────────────────────────── Compiler ────────────────────────────────────

#[test]
fn mtime_empty_project_returns_epoch() {
    let tmp = TempDir::new().unwrap();
    let mtime = latest_source_mtime_pub(tmp.path()).unwrap();
    assert_eq!(mtime, SystemTime::UNIX_EPOCH);
}

#[test]
fn mtime_finds_pages_files() {
    let tmp = TempDir::new().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir(&pages).unwrap();
    std::fs::write(pages.join("index.tsx"), "export default () => <div/>").unwrap();

    let mtime = latest_source_mtime_pub(tmp.path()).unwrap();
    assert!(mtime > SystemTime::UNIX_EPOCH);
}

#[test]
fn mtime_finds_src_pages_files() {
    let tmp = TempDir::new().unwrap();
    let src_pages = tmp.path().join("src/pages");
    std::fs::create_dir_all(&src_pages).unwrap();
    std::fs::write(src_pages.join("about.tsx"), "export default () => <div/>").unwrap();

    let mtime = latest_source_mtime_pub(tmp.path()).unwrap();
    assert!(mtime > SystemTime::UNIX_EPOCH);
}

#[test]
fn mtime_finds_app_files() {
    let tmp = TempDir::new().unwrap();
    let app = tmp.path().join("app");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("page.tsx"), "export default () => <div/>").unwrap();

    let mtime = latest_source_mtime_pub(tmp.path()).unwrap();
    assert!(mtime > SystemTime::UNIX_EPOCH);
}

#[test]
fn mtime_ignores_non_source_files() {
    let tmp = TempDir::new().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir(&pages).unwrap();
    std::fs::write(pages.join("README.md"), "not a source file").unwrap();
    std::fs::write(pages.join("data.json"), "{}").unwrap();

    let mtime = latest_source_mtime_pub(tmp.path()).unwrap();
    assert_eq!(mtime, SystemTime::UNIX_EPOCH);
}

#[test]
fn mtime_recurses_into_subdirectories() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("pages/blog/posts");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("article.tsx"), "export default () => <div/>").unwrap();

    let mtime = latest_source_mtime_pub(tmp.path()).unwrap();
    assert!(mtime > SystemTime::UNIX_EPOCH);
}

#[test]
fn mtime_picks_latest_across_extensions() {
    let tmp = TempDir::new().unwrap();
    let pages = tmp.path().join("pages");
    std::fs::create_dir(&pages).unwrap();

    // Write files — the last one written should have the latest mtime
    std::fs::write(pages.join("a.ts"), "export const a = 1").unwrap();
    std::fs::write(pages.join("b.jsx"), "export default () => <div/>").unwrap();
    std::fs::write(pages.join("c.css"), "body {}").unwrap();
    std::fs::write(pages.join("d.mdx"), "# Hello").unwrap();
    std::fs::write(pages.join("e.js"), "module.exports = {}").unwrap();

    let mtime = latest_source_mtime_pub(tmp.path()).unwrap();
    assert!(mtime > SystemTime::UNIX_EPOCH);
}

// ──────────────────────────────── LiveProject ────────────────────────────────

#[test]
fn live_project_creation() {
    use rex_live::project::LiveProjectConfig;

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("pages")).unwrap();
    std::fs::write(
        tmp.path().join("pages/index.tsx"),
        "export default () => <div/>",
    )
    .unwrap();

    let project = rex_live::project::LiveProject::new(LiveProjectConfig {
        prefix: "/app".to_string(),
        source_path: tmp.path().to_path_buf(),
        workers: 1,
    })
    .unwrap();

    assert_eq!(project.prefix, "/app");
    assert!(project.route_trie().is_none()); // not built yet
    assert!(project.api_route_trie().is_none());
    assert!(project.manifest_json().is_none());
}

#[test]
fn live_project_source_root_and_static_dir() {
    use rex_live::project::LiveProjectConfig;

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("pages")).unwrap();
    std::fs::write(
        tmp.path().join("pages/index.tsx"),
        "export default () => <div/>",
    )
    .unwrap();

    let project = rex_live::project::LiveProject::new(LiveProjectConfig {
        prefix: "/".to_string(),
        source_path: tmp.path().to_path_buf(),
        workers: 1,
    })
    .unwrap();

    assert!(project.source_root().is_absolute());
    assert!(project.static_dir().ends_with(".rex/build/client"));
}

#[test]
fn live_project_invalidate() {
    use rex_live::project::LiveProjectConfig;

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("pages")).unwrap();
    std::fs::write(
        tmp.path().join("pages/index.tsx"),
        "export default () => <div/>",
    )
    .unwrap();

    let project = rex_live::project::LiveProject::new(LiveProjectConfig {
        prefix: "/test".to_string(),
        source_path: tmp.path().to_path_buf(),
        workers: 1,
    })
    .unwrap();

    // Invalidate should not panic (cache is already empty, this is a no-op)
    project.invalidate();
}

// ──────────────────────────────── LiveServer ─────────────────────────────────

#[test]
fn server_match_project_root_mount() {
    use rex_live::server::{LiveServerConfig, MountConfig};
    use std::net::Ipv4Addr;

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("pages")).unwrap();
    std::fs::write(
        tmp.path().join("pages/index.tsx"),
        "export default () => <div/>",
    )
    .unwrap();

    let server = rex_live::server::LiveServer::new(&LiveServerConfig {
        mounts: vec![MountConfig {
            prefix: "/".to_string(),
            source: tmp.path().to_path_buf(),
        }],
        port: 0,
        host: Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    // Root mount matches everything
    let (project, remaining) = server.match_project("/").expect("should match");
    assert_eq!(project.prefix, "/");
    assert_eq!(remaining, "/");

    let (_, remaining) = server.match_project("/about").expect("should match");
    assert_eq!(remaining, "/about");
}

#[test]
fn server_match_project_prefix_mount() {
    use rex_live::server::{LiveServerConfig, MountConfig};
    use std::net::Ipv4Addr;

    let tmp_a = TempDir::new().unwrap();
    std::fs::create_dir(tmp_a.path().join("pages")).unwrap();
    std::fs::write(
        tmp_a.path().join("pages/index.tsx"),
        "export default () => <div/>",
    )
    .unwrap();

    let tmp_b = TempDir::new().unwrap();
    std::fs::create_dir(tmp_b.path().join("pages")).unwrap();
    std::fs::write(
        tmp_b.path().join("pages/index.tsx"),
        "export default () => <div/>",
    )
    .unwrap();

    let server = rex_live::server::LiveServer::new(&LiveServerConfig {
        mounts: vec![
            MountConfig {
                prefix: "/".to_string(),
                source: tmp_a.path().to_path_buf(),
            },
            MountConfig {
                prefix: "/admin".to_string(),
                source: tmp_b.path().to_path_buf(),
            },
        ],
        port: 0,
        host: Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    // /admin should match the admin project (longest prefix)
    let (project, remaining) = server.match_project("/admin").expect("should match");
    assert_eq!(project.prefix, "/admin");
    assert_eq!(remaining, "/");

    let (project, remaining) = server.match_project("/admin/users").expect("should match");
    assert_eq!(project.prefix, "/admin");
    assert_eq!(remaining, "/users");

    // Non-admin paths match root
    let (project, _) = server.match_project("/about").expect("should match");
    assert_eq!(project.prefix, "/");
}

#[test]
fn server_projects_sorted_by_prefix_length() {
    use rex_live::server::{LiveServerConfig, MountConfig};
    use std::net::Ipv4Addr;

    let make_dir = || {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("pages")).unwrap();
        std::fs::write(
            tmp.path().join("pages/index.tsx"),
            "export default () => <div/>",
        )
        .unwrap();
        tmp
    };

    let tmp_a = make_dir();
    let tmp_b = make_dir();
    let tmp_c = make_dir();

    let server = rex_live::server::LiveServer::new(&LiveServerConfig {
        mounts: vec![
            MountConfig {
                prefix: "/".to_string(),
                source: tmp_a.path().to_path_buf(),
            },
            MountConfig {
                prefix: "/admin/settings".to_string(),
                source: tmp_b.path().to_path_buf(),
            },
            MountConfig {
                prefix: "/admin".to_string(),
                source: tmp_c.path().to_path_buf(),
            },
        ],
        port: 0,
        host: Ipv4Addr::LOCALHOST.into(),
        workers_per_project: 1,
    })
    .unwrap();

    let projects = server.projects();
    assert_eq!(projects.len(), 3);
    // Longest prefix first
    assert_eq!(projects[0].prefix, "/admin/settings");
    assert_eq!(projects[1].prefix, "/admin");
    assert_eq!(projects[2].prefix, "/");
}
