use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::EMERALD;

/// Which visual state the mascot is in.
pub enum MascotState {
    /// Normal idle state — open eye with dot.
    Idle,
    /// Brief tail-flick animation after a successful rebuild.
    TailFlick,
    /// Red X eye — an unresolved error is present.
    Error,
}

// Eye character positions are the same across all variants (index 12 in line 1).
// The error variant replaces `◦` with `X`.

const IDLE: [&str; 5] = [
    "        ▄████▄       ",
    "        █ ◦ █▀█▄    ",
    "  ▄▄▄▄▄▄█████▀▀     ",
    "    ▀▀▀▀██████       ",
    "        █▀ █▀        ",
];

const TAIL_FLICK: [&str; 5] = [
    "        ▄████▄       ",
    "  ▄     █ ◦ █▀█▄    ",
    "   ▀▄▄▄▄█████▀▀     ",
    "    ▀▀▀▀██████       ",
    "        █▀ █▀        ",
];

const ERROR: [&str; 5] = [
    "        ▄████▄       ",
    "        █ X █▀█▄    ",
    "  ▄▄▄▄▄▄█████▀▀     ",
    "    ▀▀▀▀██████       ",
    "        █▀ █▀        ",
];

pub fn render_mascot(f: &mut Frame<'_>, area: Rect, state: MascotState) {
    let (mascot_lines, color) = match state {
        MascotState::Error => (&ERROR, Color::Red),
        MascotState::TailFlick => (&TAIL_FLICK, EMERALD),
        MascotState::Idle => (&IDLE, EMERALD),
    };

    let lines: Vec<Line<'_>> = mascot_lines
        .iter()
        .map(|ml| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(*ml, Style::default().fg(color)),
            ])
        })
        .collect();

    let para = Paragraph::new(lines);
    f.render_widget(para, area);
}
