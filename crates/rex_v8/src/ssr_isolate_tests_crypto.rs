use super::*;

fn make_crypto_isolate(gssp_code: &str) -> SsrIsolate {
    crate::init_v8();
    let bundle = format!(
        "{}\n{MOCK_REACT_RUNTIME}\n{}",
        rex_build::V8_POLYFILLS,
        make_server_bundle(&[(
            "page",
            &format!(
                "function Page(props) {{ return React.createElement('pre', null, JSON.stringify(props)); }}\nfunction getServerSideProps(ctx) {{ {gssp_code} }}"
            ),
            Some(&format!("function(ctx) {{ {gssp_code} }}"))
        )])
    );
    SsrIsolate::new(&bundle, None).expect("failed to create crypto isolate")
}

#[test]
fn test_crypto_random_uuid_format() {
    let mut iso = make_crypto_isolate(
        "var id = globalThis.crypto.randomUUID(); return { props: { id: id } };",
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let id = val["props"]["id"].as_str().unwrap();
    // UUID v4 format: 8-4-4-4-12 hex chars, version=4, variant=8/9/a/b
    assert_eq!(id.len(), 36, "UUID wrong length: {id}");
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 5, "UUID wrong number of parts: {id}");
    assert_eq!(
        (
            parts[0].len(),
            parts[1].len(),
            parts[2].len(),
            parts[3].len(),
            parts[4].len()
        ),
        (8, 4, 4, 4, 12),
        "UUID segment lengths wrong: {id}"
    );
    assert!(parts[2].starts_with('4'), "UUID version not 4: {id}");
    assert!(
        "89ab".contains(parts[3].chars().next().unwrap()),
        "UUID variant wrong: {id}"
    );
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "UUID has invalid chars: {id}"
    );
}

#[test]
fn test_crypto_random_uuid_uniqueness() {
    let mut iso = make_crypto_isolate(
        "var a = globalThis.crypto.randomUUID(); var b = globalThis.crypto.randomUUID(); return { props: { a: a, b: b } };",
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let a = val["props"]["a"].as_str().unwrap();
    let b = val["props"]["b"].as_str().unwrap();
    assert_ne!(a, b, "Two UUIDs should be different");
}

#[test]
fn test_crypto_create_hash_sha256_hex() {
    let mut iso = make_crypto_isolate(
        r#"var hash = globalThis.crypto.createHash('sha256').update('hello').digest('hex'); return { props: { hash: hash } };"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let hash = val["props"]["hash"].as_str().unwrap();
    assert_eq!(
        hash,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn test_crypto_create_hash_sha256_empty() {
    let mut iso = make_crypto_isolate(
        r#"var hash = globalThis.crypto.createHash('sha256').update('').digest('hex'); return { props: { hash: hash } };"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let hash = val["props"]["hash"].as_str().unwrap();
    assert_eq!(
        hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn test_crypto_create_hash_sha256_chained_updates() {
    let mut iso = make_crypto_isolate(
        r#"var hash = globalThis.crypto.createHash('sha256').update('hel').update('lo').digest('hex'); return { props: { hash: hash } };"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let hash = val["props"]["hash"].as_str().unwrap();
    assert_eq!(
        hash,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn test_crypto_create_hash_sha256_base64() {
    let mut iso = make_crypto_isolate(
        r#"var hash = globalThis.crypto.createHash('sha256').update('hello').digest('base64'); return { props: { hash: hash } };"#,
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let hash = val["props"]["hash"].as_str().unwrap();
    assert_eq!(hash, "LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=");
}

#[test]
fn test_crypto_random_bytes_length() {
    let mut iso = make_crypto_isolate(
        "var buf = globalThis.crypto.randomBytes(16); return { props: { len: buf.length } };",
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["len"].as_i64().unwrap(), 16);
}

#[test]
fn test_crypto_random_bytes_hex() {
    let mut iso = make_crypto_isolate(
        "var buf = globalThis.crypto.randomBytes(32); var hex = buf.toString('hex'); return { props: { hex: hex, len: hex.length } };",
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let hex = val["props"]["hex"].as_str().unwrap();
    assert_eq!(hex.len(), 64, "32 bytes = 64 hex chars");
    assert!(
        hex.chars().all(|c| c.is_ascii_hexdigit()),
        "should be valid hex: {hex}"
    );
}

#[test]
fn test_crypto_random_bytes_zero() {
    let mut iso = make_crypto_isolate(
        "var buf = globalThis.crypto.randomBytes(0); return { props: { len: buf.length } };",
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["len"].as_i64().unwrap(), 0);
}

#[test]
fn test_crypto_random_bytes_uniqueness() {
    let mut iso = make_crypto_isolate(
        "var a = globalThis.crypto.randomBytes(16).toString('hex'); var b = globalThis.crypto.randomBytes(16).toString('hex'); return { props: { a: a, b: b } };",
    );
    let json = iso
        .get_server_side_props("page", r#"{"params":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    let a = val["props"]["a"].as_str().unwrap();
    let b = val["props"]["b"].as_str().unwrap();
    assert_ne!(
        a, b,
        "Two randomBytes calls should produce different output"
    );
}
