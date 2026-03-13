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
    let mascot_lines = match state {
        MascotState::Error => &ERROR,
        MascotState::TailFlick => &TAIL_FLICK,
        MascotState::Idle => &IDLE,
    };

    let is_error = matches!(state, MascotState::Error);

    let lines: Vec<Line<'_>> = mascot_lines
        .iter()
        .enumerate()
        .map(|(i, ml)| {
            let mut spans = vec![Span::raw("  ")];
            // In error state, keep the body emerald but color the eye red.
            // The eye 'X' is on line index 1; split around it.
            if is_error && i == 1 {
                if let Some(x_pos) = ml.find('X') {
                    spans.push(Span::styled(&ml[..x_pos], Style::default().fg(EMERALD)));
                    spans.push(Span::styled("x", Style::default().fg(Color::Red)));
                    spans.push(Span::styled(&ml[x_pos + 1..], Style::default().fg(EMERALD)));
                } else {
                    spans.push(Span::styled(*ml, Style::default().fg(EMERALD)));
                }
            } else {
                spans.push(Span::styled(*ml, Style::default().fg(EMERALD)));
            }
            Line::from(spans)
        })
        .collect();

    let para = Paragraph::new(lines);
    f.render_widget(para, area);
}
