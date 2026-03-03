use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

// Embedded runtime files compiled into the binary via `include_str!`.
// When the binary runs on a machine without the source tree, these files
// are extracted to a temp directory so rolldown can resolve them.

// Server runtime files
const SERVER_HEAD: &str = include_str!("../../../runtime/server/head.ts");
const SERVER_LINK: &str = include_str!("../../../runtime/server/link.ts");
const SERVER_ROUTER: &str = include_str!("../../../runtime/server/router.ts");
const SERVER_DOCUMENT: &str = include_str!("../../../runtime/server/document.ts");
const SERVER_IMAGE: &str = include_str!("../../../runtime/server/image.ts");
const SERVER_MIDDLEWARE: &str = include_str!("../../../runtime/server/middleware.ts");
const SERVER_FS: &str = include_str!("../../../runtime/server/fs.ts");
const SERVER_FS_PROMISES: &str = include_str!("../../../runtime/server/fs-promises.ts");
const SERVER_PATH: &str = include_str!("../../../runtime/server/path.ts");

// Client runtime files
const CLIENT_LINK: &str = include_str!("../../../runtime/client/link.ts");
const CLIENT_HEAD: &str = include_str!("../../../runtime/client/head.ts");
const CLIENT_USE_ROUTER: &str = include_str!("../../../runtime/client/use-router.ts");
const CLIENT_IMAGE: &str = include_str!("../../../runtime/client/image.ts");

const SERVER_FILES: &[(&str, &str)] = &[
    ("head.ts", SERVER_HEAD),
    ("link.ts", SERVER_LINK),
    ("router.ts", SERVER_ROUTER),
    ("document.ts", SERVER_DOCUMENT),
    ("image.ts", SERVER_IMAGE),
    ("middleware.ts", SERVER_MIDDLEWARE),
    ("fs.ts", SERVER_FS),
    ("fs-promises.ts", SERVER_FS_PROMISES),
    ("path.ts", SERVER_PATH),
];

const CLIENT_FILES: &[(&str, &str)] = &[
    ("link.ts", CLIENT_LINK),
    ("head.ts", CLIENT_HEAD),
    ("use-router.ts", CLIENT_USE_ROUTER),
    ("image.ts", CLIENT_IMAGE),
];

static EXTRACTED: OnceLock<PathBuf> = OnceLock::new();

/// Extract all embedded runtime files to a temp directory.
/// The directory is reused for the lifetime of the process.
pub fn extract() -> Result<PathBuf> {
    if let Some(dir) = EXTRACTED.get() {
        return Ok(dir.clone());
    }

    let base = std::env::temp_dir().join(format!("rex-runtime-{}", std::process::id()));
    let server_dir = base.join("server");
    let client_dir = base.join("client");

    fs::create_dir_all(&server_dir)?;
    fs::create_dir_all(&client_dir)?;

    for (name, content) in SERVER_FILES {
        fs::write(server_dir.join(name), content)?;
    }
    for (name, content) in CLIENT_FILES {
        fs::write(client_dir.join(name), content)?;
    }

    // Store and return; ignore race — OnceLock ensures only one value is kept.
    let _ = EXTRACTED.set(base.clone());
    Ok(base)
}

pub fn server_dir() -> Result<PathBuf> {
    Ok(extract()?.join("server"))
}

pub fn client_dir() -> Result<PathBuf> {
    Ok(extract()?.join("client"))
}
