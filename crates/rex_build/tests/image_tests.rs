#![allow(clippy::unwrap_used)]

mod common;

use common::setup_test_project;

#[tokio::test]
async fn test_image_object_src_extracts_string() {
    // Verify that the Image component handles { src, width, height } objects.
    // This simulates what StaticAssetPlugin returns for static image imports.
    // The Image component must extract .src from the object instead of
    // stringifying it as [object Object].
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import Image from 'rex/image';
                const logo = { src: "/_rex/static/assets/logo-abc123.png", height: 100, width: 200 };
                export default function Home() {
                    return <div><Image src={logo} alt="Logo" width={200} height={100} /></div>;
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = common::build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .expect("pool execute")
        .expect("render_page");

    // The rendered img src should contain the extracted URL going through the
    // image optimizer (PNGs are raster, so they get optimized)
    assert!(
        render
            .body
            .contains("/_rex/image?url=%2F_rex%2Fstatic%2Fassets%2Flogo-abc123.png"),
        "Image src should extract .src from static import object and route through optimizer: {}",
        render.body
    );
    assert!(
        !render.body.contains("[object Object]"),
        "Image src should not contain [object Object]: {}",
        render.body
    );
}

#[tokio::test]
async fn test_image_svg_skips_optimizer() {
    // SVG images should be served directly, not routed through /_rex/image
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import Image from 'rex/image';
                const icon = { src: "/_rex/static/assets/icon-abc.svg", height: 24, width: 24 };
                export default function Home() {
                    return <div><Image src={icon} alt="Icon" width={24} height={24} /></div>;
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = common::build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .expect("pool execute")
        .expect("render_page");

    // SVG should NOT go through the image optimizer
    assert!(
        !render.body.contains("/_rex/image?"),
        "SVG should not be routed through the image optimizer: {}",
        render.body
    );
    // SVG should use direct static asset URL
    assert!(
        render.body.contains("/_rex/static/assets/icon-abc.svg"),
        "SVG should use direct static asset URL: {}",
        render.body
    );
}

#[tokio::test]
async fn test_image_string_src_unchanged() {
    // Plain string src should still work (normal public/ path usage)
    let (_tmp, config, scan) = setup_test_project(
        &[(
            "index.tsx",
            r#"
                import Image from 'rex/image';
                export default function Home() {
                    return <div><Image src="/images/hero.jpg" alt="Hero" width={800} height={600} /></div>;
                }
                "#,
        )],
        None,
    );

    let (_result, pool) = common::build_and_load(&config, &scan).await;

    let render = pool
        .execute(|iso| iso.render_page("index", "{}"))
        .await
        .expect("pool execute")
        .expect("render_page");

    // String src should go through the image optimizer as before
    assert!(
        render.body.contains("/_rex/image?url=%2Fimages%2Fhero.jpg"),
        "String src should use the image optimizer: {}",
        render.body
    );
}
