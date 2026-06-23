#![allow(clippy::unwrap_used)]

mod common;

use common::setup_test_project;
use rex_build::build_bundles;
use rex_core::ProjectConfig;
use std::fs;

/// Test that React is isolated into vendor chunk(s) with content-based hashes.
///
/// Regression test for HMR crash: when React was co-bundled into shared app chunks,
/// HMR would re-initialize React, breaking the running component tree with
/// "Cannot read properties of null (reading 'useState')".
#[tokio::test]
async fn test_vendor_chunk_isolates_react() {
    let (_tmp, config, scan) = setup_test_project(
        &[
            (
                "index.tsx",
                "export default function Home() { return <div>Home</div>; }",
            ),
            (
                "about.tsx",
                "export default function About() { return <div>About</div>; }",
            ),
        ],
        None,
    );
    let result = build_bundles(&config, &scan, &ProjectConfig::default())
        .await
        .unwrap();

    let client_dir = config.client_build_dir();
    let build_hash = &result.build_id[..8];

    // Find vendor chunks in the output directory (rolldown may split react
    // and react-dom into separate vendor chunks)
    let vendor_chunks: Vec<_> = fs::read_dir(&client_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("chunk-vendor-"))
        .collect();
    assert!(
        !vendor_chunks.is_empty(),
        "should have at least 1 vendor chunk"
    );

    // At least one vendor chunk should contain React's createElement
    let vendor_has_react = vendor_chunks.iter().any(|c| {
        fs::read_to_string(c.path())
            .unwrap()
            .contains("createElement")
    });
    assert!(vendor_has_react, "vendor chunk(s) should contain React");

    // Vendor chunks should use content-based hash, NOT the build_id hash
    for chunk in &vendor_chunks {
        let name = chunk.file_name().to_string_lossy().to_string();
        assert!(
            !name.contains(build_hash),
            "vendor chunk should use content hash, not build_id hash: {name}"
        );
    }

    // Page entry chunks should NOT contain React inline
    let index_js = fs::read_to_string(client_dir.join(format!("index-{build_hash}.js"))).unwrap();
    assert!(
        !index_js.contains("function createElement"),
        "page chunk should not bundle React inline — it should import from vendor chunk"
    );

    // Page entry chunks should import from the vendor chunk
    assert!(
        index_js.contains("chunk-vendor-"),
        "page chunk should import from vendor chunk"
    );

    // Manifest should list vendor chunk(s) as shared chunks
    assert!(
        result
            .manifest
            .shared_chunks
            .iter()
            .any(|c| c.starts_with("chunk-vendor-")),
        "manifest should track vendor as a shared chunk"
    );
}
