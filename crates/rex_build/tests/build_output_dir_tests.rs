//! Regression tests for builds that overlap on the same output directory.
//!
//! In dev, the bundler's output dir is shared across builds. A `rex dev` init
//! build can be cancelled by a dropped HTTP connection and then restarted (see
//! `rex_server`'s lazy init), so two builds can be in flight against the same
//! `.rex/build` dir at once. If a build wiped the whole output dir on start, or
//! removed a fixed (shared) scratch dir on finish, one build's cleanup would
//! delete another build's in-flight files mid-bundle — which surfaced to
//! rolldown as `[UNLOADABLE_DEPENDENCY] Could not load .rex/build/client/_dce/
//! <page>.tsx` and bubbled up as a flaky 500 / failing e2e test.
//!
//! The fix has two parts, each verified deterministically here:
//!  1. Dev builds do NOT wipe the shared output dir (rely on content-hashed
//!     names instead) — so a concurrent build's files survive. (Unit coverage of
//!     the per-build scratch dirs lives in `build_utils`'s inline tests.)
//!  2. Production builds, which are single-shot with no concurrency, still wipe.
#![allow(clippy::unwrap_used)]

mod common;

use common::setup_test_project;
use rex_core::ProjectConfig;
use std::fs;

const PAGES: &[(&str, &str)] = &[
    (
        "index.tsx",
        "export default function Home() { return <div>Home</div>; }",
    ),
    // A dynamic page WITH a server export, so DCE writes a stripped
    // `_dce/posts-_id_.tsx` scratch file — mirroring the page that failed in CI.
    (
        "posts/[id].tsx",
        r#"
        export async function getServerSideProps() {
            return { props: { id: "x" } };
        }
        export default function Post({ id }) { return <div>{id}</div>; }
        "#,
    ),
];

/// A dev build must leave files written by a concurrent (overlapping) build
/// alone — i.e. it must not wipe the shared output dir on start. Modeled by
/// dropping a sentinel into the client output dir before building and asserting
/// it survives. Before the fix, the start-of-build `remove_dir_all` deleted it.
#[tokio::test]
async fn dev_build_does_not_wipe_shared_output_dir() {
    let (_tmp, config, scan) = setup_test_project(PAGES, None);
    assert!(config.dev, "this regression covers the dev build path");

    // Simulate a concurrently-running build's artifact already in the output dir.
    let client_dir = config.client_build_dir();
    fs::create_dir_all(&client_dir).unwrap();
    let sentinel = client_dir.join("_concurrent_build_artifact.js");
    fs::write(&sentinel, "// another build's in-flight file").unwrap();

    build_bundles(&config, &scan).await;

    assert!(
        sentinel.exists(),
        "dev build wiped the shared output dir, clobbering a concurrent build's file"
    );
}

/// Production builds are single-shot, so they keep wiping stale artifacts.
#[tokio::test]
async fn production_build_wipes_stale_output_dir() {
    let (_tmp, mut config, scan) = setup_test_project(PAGES, None);
    config = config.with_dev(false);

    let client_dir = config.client_build_dir();
    fs::create_dir_all(&client_dir).unwrap();
    let stale = client_dir.join("_stale_artifact.js");
    fs::write(&stale, "// stale output from a previous build").unwrap();

    build_bundles(&config, &scan).await;

    assert!(
        !stale.exists(),
        "production build should wipe stale artifacts from the output dir"
    );
}

async fn build_bundles(config: &rex_core::RexConfig, scan: &rex_router::ScanResult) {
    rex_build::build_bundles(config, scan, &ProjectConfig::default())
        .await
        .unwrap();
}
