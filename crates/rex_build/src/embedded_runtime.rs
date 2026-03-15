use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

// Embedded runtime files compiled into the binary via `include_str!`.
// When the binary runs on a machine without the source tree, these files
// are extracted to a temp directory so rolldown can resolve them.

const SERVER_FILES: &[(&str, &str)] = &[
    (
        "actions.ts",
        include_str!("../../../runtime/server/actions.ts"),
    ),
    (
        "app-route-runtime.ts",
        include_str!("../../../runtime/server/app-route-runtime.ts"),
    ),
    (
        "assert.ts",
        include_str!("../../../runtime/server/assert.ts"),
    ),
    (
        "buffer.ts",
        include_str!("../../../runtime/server/buffer.ts"),
    ),
    (
        "child_process.ts",
        include_str!("../../../runtime/server/child_process.ts"),
    ),
    (
        "cloudflare-sockets.ts",
        include_str!("../../../runtime/server/cloudflare-sockets.ts"),
    ),
    (
        "crypto.ts",
        include_str!("../../../runtime/server/crypto.ts"),
    ),
    ("dns.ts", include_str!("../../../runtime/server/dns.ts")),
    (
        "document.ts",
        include_str!("../../../runtime/server/document.ts"),
    ),
    ("empty.ts", include_str!("../../../runtime/server/empty.ts")),
    (
        "events.cjs",
        include_str!("../../../runtime/server/events.cjs"),
    ),
    (
        "events.ts",
        include_str!("../../../runtime/server/events.ts"),
    ),
    (
        "file-type.ts",
        include_str!("../../../runtime/server/file-type.ts"),
    ),
    (
        "fs-promises.ts",
        include_str!("../../../runtime/server/fs-promises.ts"),
    ),
    ("fs.ts", include_str!("../../../runtime/server/fs.ts")),
    ("head.ts", include_str!("../../../runtime/server/head.ts")),
    ("http.ts", include_str!("../../../runtime/server/http.ts")),
    ("http2.ts", include_str!("../../../runtime/server/http2.ts")),
    ("https.ts", include_str!("../../../runtime/server/https.ts")),
    ("image.ts", include_str!("../../../runtime/server/image.ts")),
    ("link.ts", include_str!("../../../runtime/server/link.ts")),
    (
        "mcp-runtime.ts",
        include_str!("../../../runtime/server/mcp-runtime.ts"),
    ),
    (
        "metadata.ts",
        include_str!("../../../runtime/server/metadata.ts"),
    ),
    (
        "middleware-runtime.ts",
        include_str!("../../../runtime/server/middleware-runtime.ts"),
    ),
    (
        "middleware.ts",
        include_str!("../../../runtime/server/middleware.ts"),
    ),
    (
        "module.ts",
        include_str!("../../../runtime/server/module.ts"),
    ),
    ("net.ts", include_str!("../../../runtime/server/net.ts")),
    (
        "next-cache.ts",
        include_str!("../../../runtime/server/next-cache.ts"),
    ),
    (
        "next-dynamic.ts",
        include_str!("../../../runtime/server/next-dynamic.ts"),
    ),
    (
        "next-font.ts",
        include_str!("../../../runtime/server/next-font.ts"),
    ),
    (
        "next-headers.ts",
        include_str!("../../../runtime/server/next-headers.ts"),
    ),
    (
        "next-image.ts",
        include_str!("../../../runtime/server/next-image.ts"),
    ),
    (
        "next-link.ts",
        include_str!("../../../runtime/server/next-link.ts"),
    ),
    (
        "next-navigation.ts",
        include_str!("../../../runtime/server/next-navigation.ts"),
    ),
    (
        "next-router.ts",
        include_str!("../../../runtime/server/next-router.ts"),
    ),
    (
        "next-server.ts",
        include_str!("../../../runtime/server/next-server.ts"),
    ),
    ("os.ts", include_str!("../../../runtime/server/os.ts")),
    ("path.ts", include_str!("../../../runtime/server/path.ts")),
    (
        "process.ts",
        include_str!("../../../runtime/server/process.ts"),
    ),
    (
        "querystring.ts",
        include_str!("../../../runtime/server/querystring.ts"),
    ),
    (
        "react-dom-server-stub.ts",
        include_str!("../../../runtime/server/react-dom-server-stub.ts"),
    ),
    (
        "react-group-shim.ts",
        include_str!("../../../runtime/server/react-group-shim.ts"),
    ),
    (
        "react-jsx-dev-group-shim.ts",
        include_str!("../../../runtime/server/react-jsx-dev-group-shim.ts"),
    ),
    (
        "react-jsx-group-shim.ts",
        include_str!("../../../runtime/server/react-jsx-group-shim.ts"),
    ),
    (
        "react-server-bridge.ts",
        include_str!("../../../runtime/server/react-server-bridge.ts"),
    ),
    (
        "readline.ts",
        include_str!("../../../runtime/server/readline.ts"),
    ),
    (
        "router.ts",
        include_str!("../../../runtime/server/router.ts"),
    ),
    ("sharp.ts", include_str!("../../../runtime/server/sharp.ts")),
    (
        "ssr-runtime.ts",
        include_str!("../../../runtime/server/ssr-runtime.ts"),
    ),
    (
        "stream-web.ts",
        include_str!("../../../runtime/server/stream-web.ts"),
    ),
    (
        "stream.ts",
        include_str!("../../../runtime/server/stream.ts"),
    ),
    (
        "string_decoder.ts",
        include_str!("../../../runtime/server/string_decoder.ts"),
    ),
    ("tls.ts", include_str!("../../../runtime/server/tls.ts")),
    ("tty.ts", include_str!("../../../runtime/server/tty.ts")),
    (
        "url-module.ts",
        include_str!("../../../runtime/server/url-module.ts"),
    ),
    ("url.ts", include_str!("../../../runtime/server/url.ts")),
    ("util.ts", include_str!("../../../runtime/server/util.ts")),
    (
        "worker_threads.ts",
        include_str!("../../../runtime/server/worker_threads.ts"),
    ),
    ("zlib.ts", include_str!("../../../runtime/server/zlib.ts")),
];

const CLIENT_FILES: &[(&str, &str)] = &[
    ("head.ts", include_str!("../../../runtime/client/head.ts")),
    ("image.ts", include_str!("../../../runtime/client/image.ts")),
    ("link.ts", include_str!("../../../runtime/client/link.ts")),
    (
        "router.ts",
        include_str!("../../../runtime/client/router.ts"),
    ),
    (
        "rsc-hydrate.ts",
        include_str!("../../../runtime/client/rsc-hydrate.ts"),
    ),
    (
        "rsc-runtime.ts",
        include_str!("../../../runtime/client/rsc-runtime.ts"),
    ),
    (
        "use-router.ts",
        include_str!("../../../runtime/client/use-router.ts"),
    ),
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
