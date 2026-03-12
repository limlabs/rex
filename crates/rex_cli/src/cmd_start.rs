use anyhow::Result;
use std::net::IpAddr;
use std::path::PathBuf;

use crate::display::*;

pub(crate) async fn cmd_start(root: PathBuf, port: u16, host: IpAddr) -> Result<()> {
    let start = std::time::Instant::now();

    print_mascot_header(env!("CARGO_PKG_VERSION"), "(production)");

    let rex = rex_server::Rex::from_build(rex_server::RexOptions {
        root,
        dev: false,
        port,
        host,
    })
    .await?;

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
    print_route_summary(&rex.scan().routes, &rex.scan().api_routes);
    eprintln!();

    rex.serve().await
}
