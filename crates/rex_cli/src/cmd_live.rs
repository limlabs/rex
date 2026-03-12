use anyhow::Result;
use rex_live::server::{LiveServerConfig, MountConfig};
use std::net::IpAddr;

use crate::display::*;

pub(crate) async fn cmd_live(
    mount: Vec<String>,
    port: u16,
    host: IpAddr,
    workers: usize,
) -> Result<()> {
    let start = std::time::Instant::now();

    print_mascot_header(env!("CARGO_PKG_VERSION"), "(live)");

    if mount.is_empty() {
        anyhow::bail!("No mounts specified. Use --mount PREFIX=SOURCE (e.g., --mount /=./my-app)");
    }

    let mut mounts = Vec::new();
    for m in &mount {
        let (prefix, source) = m
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid mount format: {m}. Expected PREFIX=SOURCE"))?;

        let source_path = std::fs::canonicalize(source)
            .map_err(|e| anyhow::anyhow!("Source path not found: {source}: {e}"))?;

        mounts.push(MountConfig {
            prefix: prefix.to_string(),
            source: source_path,
        });
    }

    let config = LiveServerConfig {
        mounts,
        port,
        host,
        workers_per_project: workers,
    };

    let server = rex_live::server::LiveServer::new(&config)?;

    let elapsed = start.elapsed();
    eprintln!(
        "  {} {}",
        green_bold("✓ Ready in"),
        green_bold(&format_duration(elapsed))
    );
    eprintln!();
    eprintln!(
        "  {} {}",
        dim("➜ Local:"),
        bold(&format!("http://localhost:{port}"))
    );
    eprintln!();
    for project in server.projects() {
        eprintln!(
            "  {} {} → {}",
            dim("◇"),
            bold(&project.prefix),
            dim(&project.source_root().display().to_string()),
        );
    }
    eprintln!();
    eprintln!(
        "  {}",
        dim("Projects compile on first request. File changes auto-invalidate cache.")
    );
    eprintln!();

    server.serve(port, host).await
}
