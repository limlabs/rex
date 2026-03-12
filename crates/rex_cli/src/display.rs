pub(crate) fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[0m")
}

pub(crate) fn dim(s: &str) -> String {
    format!("\x1b[2m{s}\x1b[0m")
}

pub(crate) fn magenta_bold(s: &str) -> String {
    format!("\x1b[1;35m{s}\x1b[0m")
}

pub(crate) fn green_bold(s: &str) -> String {
    format!("\x1b[1;32m{s}\x1b[0m")
}

fn emerald(s: &str) -> String {
    format!("\x1b[38;2;46;204;113m{s}\x1b[0m")
}

pub(crate) fn print_mascot_header(version: &str, suffix: &str) {
    // Rex mascot - each line padded to 20 display chars
    let m: [&str; 5] = [
        "        ▄████▄       ",
        "        █ ◦ █▀█▄    ",
        "  ▄▄▄▄▄▄█████▀▀     ",
        "    ▀▀▀▀██████       ",
        "        █▀ █▀        ",
    ];
    let version_line = if suffix.is_empty() {
        format!("{} {}", emerald("rex"), dim(version))
    } else {
        format!("{} {} {}", emerald("rex"), dim(version), dim(suffix))
    };
    eprintln!();
    eprintln!("  {}", emerald(m[0]));
    eprintln!("  {}  {}", emerald(m[1]), version_line);
    eprintln!("  {}", emerald(m[2]));
    eprintln!("  {}", emerald(m[3]));
    eprintln!("  {}", emerald(m[4]));
    eprintln!();
}

pub(crate) fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms >= 1000 {
        format!("{:.2}s", d.as_secs_f64())
    } else {
        format!("{ms}ms")
    }
}

pub(crate) fn print_route_summary(routes: &[rex_core::Route], api_routes: &[rex_core::Route]) {
    let page_count = routes.len();
    let api_count = api_routes.len();

    let mut parts = Vec::new();
    if page_count > 0 {
        parts.push(format!(
            "{} {}",
            page_count,
            if page_count == 1 { "page" } else { "pages" }
        ));
    }
    if api_count > 0 {
        parts.push(format!(
            "{} API {}",
            api_count,
            if api_count == 1 { "route" } else { "routes" }
        ));
    }

    if !parts.is_empty() {
        eprintln!("  {}", dim(&parts.join(" · ")));
    }
}
