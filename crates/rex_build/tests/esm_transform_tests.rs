//! Integration tests for `esm_transform::transform_for_browser`.
#![allow(clippy::unwrap_used)]

use rex_build::esm_transform::{dep_specifiers, transform_for_browser};

#[test]
fn transform_for_browser_strips_gssp() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();
    let file_path = project_root.join("pages/index.tsx");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    let source = r#"
import React from 'react';

export function getServerSideProps() {
    return { props: { name: "world" } };
}

export default function Home({ name }: { name: string }) {
    return <div>Hello {name}</div>;
}
"#;
    std::fs::write(&file_path, source).unwrap();

    let known = dep_specifiers(false);
    let result = transform_for_browser(source, &file_path, project_root, &known).unwrap();
    assert!(
        !result.contains("getServerSideProps"),
        "DCE should strip getServerSideProps, got: {result}"
    );
    assert!(result.contains("Home"), "Default export should survive DCE");
}

#[test]
fn transform_for_browser_strips_types() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();
    let file_path = project_root.join("pages/typed.tsx");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    let source = r#"
interface Props {
    count: number;
}
export default function Counter(props: Props) {
    const x: number = props.count;
    return <span>{x}</span>;
}
"#;
    std::fs::write(&file_path, source).unwrap();

    let known = dep_specifiers(false);
    let result = transform_for_browser(source, &file_path, project_root, &known).unwrap();
    assert!(
        !result.contains("interface Props"),
        "TypeScript interface should be stripped, got: {result}"
    );
    assert!(
        !result.contains(": number"),
        "Type annotations should be stripped, got: {result}"
    );
    assert!(
        result.contains("Counter"),
        "Function name should survive, got: {result}"
    );
}

#[test]
fn transform_for_browser_rewrites_imports_to_urls() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();

    // Create the source file and the imported file so resolution works
    let pages_dir = project_root.join("pages");
    let components_dir = project_root.join("components");
    std::fs::create_dir_all(&pages_dir).unwrap();
    std::fs::create_dir_all(&components_dir).unwrap();

    let button_path = components_dir.join("Button.tsx");
    std::fs::write(
        &button_path,
        "export default function Button() { return null; }\n",
    )
    .unwrap();

    let file_path = pages_dir.join("index.tsx");
    let source = "import Button from '../components/Button';\nexport default function Home() { return <Button />; }\n";
    std::fs::write(&file_path, source).unwrap();

    let known = dep_specifiers(false);
    let result = transform_for_browser(source, &file_path, project_root, &known).unwrap();
    assert!(
        result.contains("/_rex/src/"),
        "Relative imports should be rewritten to /_rex/src/ URLs, got: {result}"
    );
    assert!(
        !result.contains("'../components/Button'"),
        "Original relative specifier should be replaced, got: {result}"
    );
}
