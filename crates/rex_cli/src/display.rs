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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_wraps_text() {
        assert_eq!(bold("hi"), "\x1b[1mhi\x1b[0m");
    }

    #[test]
    fn dim_wraps_text() {
        assert_eq!(dim("hi"), "\x1b[2mhi\x1b[0m");
    }

    #[test]
    fn magenta_bold_wraps_text() {
        assert_eq!(magenta_bold("hi"), "\x1b[1;35mhi\x1b[0m");
    }

    #[test]
    fn green_bold_wraps_text() {
        assert_eq!(green_bold("hi"), "\x1b[1;32mhi\x1b[0m");
    }

    #[test]
    fn emerald_wraps_text() {
        assert_eq!(emerald("hi"), "\x1b[38;2;46;204;113mhi\x1b[0m");
    }

    #[test]
    fn format_duration_milliseconds() {
        let d = std::time::Duration::from_millis(750);
        assert_eq!(format_duration(d), "750ms");
    }

    #[test]
    fn format_duration_seconds() {
        let d = std::time::Duration::from_millis(1500);
        assert_eq!(format_duration(d), "1.50s");
    }

    #[test]
    fn format_duration_zero() {
        let d = std::time::Duration::from_millis(0);
        assert_eq!(format_duration(d), "0ms");
    }
}
