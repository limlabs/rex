pub mod log_layer;
mod logs;
mod mascot;
mod text;

use crate::tui::log_layer::LogBuffer;
use crate::tui::mascot::{render_mascot, MascotState};
use crate::tui::text::{wrap_text, wrapped_line_count};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;
use std::collections::HashSet;
use std::time::Duration;
use tracing::Level;

const EMERALD: Color = Color::Rgb(46, 204, 113);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Logs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFilter {
    All,
    Error,
    Warn,
    Info,
    Debug,
}

impl LogFilter {
    fn matches(self, level: &Level) -> bool {
        match self {
            LogFilter::All => true,
            LogFilter::Error => *level == Level::ERROR,
            LogFilter::Warn => *level <= Level::WARN,
            LogFilter::Info => *level <= Level::INFO,
            LogFilter::Debug => true,
        }
    }

    fn label(self) -> &'static str {
        match self {
            LogFilter::All => "All",
            LogFilter::Error => "Error",
            LogFilter::Warn => "Warn",
            LogFilter::Info => "Info",
            LogFilter::Debug => "Debug",
        }
    }
}

/// Info collected after server startup, displayed on the home screen.
pub struct StartupInfo {
    pub version: String,
    pub ready_ms: u64,
    pub url: String,
    pub page_count: usize,
    pub api_count: usize,
    pub has_tailwind: bool,
    pub has_typescript: bool,
}

/// An unresolved error with its full message (may contain a stack trace).
struct UnresolvedError {
    message: String,
}

struct TuiApp {
    pub(crate) screen: Screen,
    pub(crate) log_buffer: LogBuffer,
    pub(crate) startup_info: StartupInfo,
    pub(crate) log_filter: LogFilter,
    pub(crate) log_scroll: usize,
    pub(crate) search_query: String,
    pub(crate) searching: bool,
    pub(crate) auto_scroll: bool,
    /// Countdown ticks for the tail-flick animation on HMR rebuild.
    tail_flick_ticks: u8,
    /// Log counter at last tick, used to detect new entries.
    last_seen_count: usize,
    /// Currently unresolved error. Set on ERROR log, cleared on successful rebuild.
    unresolved_error: Option<UnresolvedError>,
    /// Full-screen error view toggle (Ctrl+O).
    error_expanded: bool,
    /// Scroll offset within the expanded error view.
    error_scroll: usize,
    /// Selected entry index in the log list (for Space-to-expand).
    pub(crate) log_cursor: usize,
    /// Set of log entry indices (in current filtered list) that are expanded.
    pub(crate) expanded_entries: HashSet<usize>,
}

impl TuiApp {
    fn new(log_buffer: LogBuffer, startup_info: StartupInfo) -> Self {
        let last_seen_count = log_buffer.total_count();
        Self {
            screen: Screen::Home,
            log_buffer,
            startup_info,
            log_filter: LogFilter::All,
            log_scroll: 0,
            search_query: String::new(),
            searching: false,
            auto_scroll: true,
            tail_flick_ticks: 0,
            last_seen_count,
            unresolved_error: None,
            error_expanded: false,
            error_scroll: 0,
            log_cursor: 0,
            expanded_entries: HashSet::new(),
        }
    }

    /// Advance animation state and detect HMR rebuilds.
    fn tick(&mut self) {
        let (new_entries, new_count) = self.log_buffer.drain_since(self.last_seen_count);
        self.last_seen_count = new_count;

        for entry in new_entries {
            if entry.message.starts_with("Rebuild complete") {
                self.tail_flick_ticks = 4;
                self.unresolved_error = None;
                self.error_expanded = false;
            } else if entry.level == Level::ERROR {
                self.unresolved_error = Some(UnresolvedError {
                    message: entry.message,
                });
            }
        }

        self.tail_flick_ticks = self.tail_flick_ticks.saturating_sub(1);
    }

    fn mascot_state(&self) -> MascotState {
        if self.unresolved_error.is_some() {
            MascotState::Error
        } else if self.tail_flick_ticks > 0 {
            MascotState::TailFlick
        } else {
            MascotState::Idle
        }
    }

    fn render(&mut self, f: &mut Frame<'_>) {
        match self.screen {
            Screen::Home => self.render_home(f),
            Screen::Logs => self.render_logs(f),
        }
    }

    fn render_home(&mut self, f: &mut Frame<'_>) {
        let area = f.area();

        // Full-screen expanded error view
        if self.error_expanded {
            if let Some(ref err) = self.unresolved_error {
                Self::render_expanded_error(f, area, &err.message, &mut self.error_scroll);
                return;
            }
            self.error_expanded = false;
        }

        // 2-char indent prefix on each error line
        let err_width = area.width.saturating_sub(2) as usize;
        let error_lines = self
            .unresolved_error
            .as_ref()
            .map(|e| {
                let visual: usize = e
                    .message
                    .lines()
                    .take(12)
                    .map(|l| wrapped_line_count(l, err_width))
                    .sum();
                visual as u16 + 2
            })
            .unwrap_or(0);

        let chunks = Layout::vertical([
            Constraint::Length(2),           // top padding
            Constraint::Length(7),           // mascot + info + last log
            Constraint::Length(error_lines), // error stack trace (0 when no error)
            Constraint::Min(0),              // spacer
            Constraint::Length(1),           // footer
        ])
        .split(area);

        self.render_mascot_and_info(f, chunks[1]);
        if let Some(ref err) = self.unresolved_error {
            Self::render_error_section(f, chunks[2], err);
        }
        self.render_home_footer(f, chunks[4]);
    }

    fn render_expanded_error(f: &mut Frame<'_>, area: Rect, message: &str, scroll: &mut usize) {
        let chunks = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Min(1),    // content
            Constraint::Length(1), // footer
        ])
        .split(area);

        // Header
        let header = Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "✕ Error (expanded)",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]);
        f.render_widget(Paragraph::new(header), chunks[0]);

        // Content with scroll
        let width = chunks[1].width.saturating_sub(2) as usize;
        let lines: Vec<Line<'_>> = message
            .lines()
            .flat_map(|l| {
                wrap_text(l, width).into_iter().map(|s| {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(s, Style::default().fg(Color::Red)),
                    ])
                })
            })
            .collect();

        let total = lines.len();
        let visible = chunks[1].height as usize;
        *scroll = (*scroll).min(total.saturating_sub(visible));
        let para = Paragraph::new(lines).scroll((*scroll as u16, 0));
        f.render_widget(para, chunks[1]);

        // Footer
        let footer = Line::from(vec![
            Span::raw("  "),
            Span::styled("j/k", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" scroll", Style::default().fg(Color::DarkGray)),
            Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Ctrl+O", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" close", Style::default().fg(Color::DarkGray)),
            Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
            Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(footer), chunks[2]);
    }

    fn render_mascot_and_info(&self, f: &mut Frame<'_>, area: Rect) {
        let cols = Layout::horizontal([Constraint::Length(25), Constraint::Min(0)]).split(area);
        render_mascot(f, cols[0], self.mascot_state());
        self.render_info(f, cols[1]);
    }

    fn render_error_section(f: &mut Frame<'_>, area: Rect, err: &UnresolvedError) {
        let mut lines: Vec<Line<'_>> = Vec::new();
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "✕ Error",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]));
        let max_lines = 12;
        let all_lines: Vec<&str> = err.message.lines().collect();
        let show = all_lines.len().min(max_lines);
        for line in &all_lines[..show] {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(*line, Style::default().fg(Color::Red)),
            ]));
        }
        if all_lines.len() > max_lines {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "  ... Ctrl+O to expand",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(para, area);
    }

    fn render_info(&self, f: &mut Frame<'_>, area: Rect) {
        let info = &self.startup_info;

        let mut route_parts = Vec::new();
        if info.page_count > 0 {
            let label = if info.page_count == 1 {
                "page"
            } else {
                "pages"
            };
            route_parts.push(format!("{} {label}", info.page_count));
        }
        if info.api_count > 0 {
            let label = if info.api_count == 1 {
                "route"
            } else {
                "routes"
            };
            route_parts.push(format!("{} API {label}", info.api_count));
        }
        let route_summary = route_parts.join(" · ");

        let mut features = Vec::new();
        if info.has_tailwind {
            features.push("Tailwind CSS");
        }
        if info.has_typescript {
            features.push("TypeScript");
        }
        let feature_line = features
            .iter()
            .map(|f| format!("◇ {f}"))
            .collect::<Vec<_>>()
            .join(" · ");

        let has_error = self.unresolved_error.is_some();
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    "rex ",
                    Style::default().fg(EMERALD).add_modifier(Modifier::BOLD),
                ),
                Span::styled(&info.version, Style::default().fg(Color::DarkGray)),
            ]),
            if has_error {
                Line::from(vec![Span::styled(
                    "✕ Error",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )])
            } else {
                Line::from(vec![Span::styled(
                    format!("✓ Ready in {}ms", info.ready_ms),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )])
            },
            Line::from(vec![
                Span::styled("➜ Local: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&info.url, Style::default().add_modifier(Modifier::BOLD)),
            ]),
        ];

        let mut route_spans = vec![Span::styled(
            &route_summary,
            Style::default().fg(Color::DarkGray),
        )];
        if !feature_line.is_empty() {
            route_spans.push(Span::styled(
                format!(" · {feature_line}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(route_spans));

        // Blank line then last log
        lines.push(Line::from(""));
        let last_entry = self.log_buffer.last();
        let is_dup = matches!(
            (&last_entry, &self.unresolved_error),
            (Some(e), Some(err)) if e.message == err.message
        );
        lines.push(if is_dup {
            Line::from("")
        } else {
            match last_entry {
                Some(entry) => Line::from(vec![
                    Span::styled(
                        level_symbol(&entry.level),
                        Style::default().fg(level_color(&entry.level)),
                    ),
                    Span::raw(" "),
                    Span::styled(entry.message, Style::default().fg(Color::DarkGray)),
                ]),
                None => Line::from(Span::styled(
                    "Waiting for activity...",
                    Style::default().fg(Color::DarkGray),
                )),
            }
        });

        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }

    fn render_home_footer(&self, f: &mut Frame<'_>, area: Rect) {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled("l", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" Logs", Style::default().fg(Color::DarkGray)),
        ];
        if self.unresolved_error.is_some() {
            spans.extend([
                Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Ctrl+O", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(" expand error", Style::default().fg(Color::DarkGray)),
            ]);
        }
        spans.extend([
            Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
            Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => self.handle_key(key),
            _ => false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }

        if self.searching {
            return self.handle_search_key(key);
        }

        match self.screen {
            Screen::Home => self.handle_home_key(key),
            Screen::Logs => self.handle_logs_key(key),
        }
    }

    fn handle_home_key(&mut self, key: KeyEvent) -> bool {
        // Expanded error mode: scroll or close
        if self.error_expanded {
            return match key.code {
                KeyCode::Char('q') => true,
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.error_expanded = false;
                    false
                }
                KeyCode::Esc => {
                    self.error_expanded = false;
                    false
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.error_scroll = self.error_scroll.saturating_add(1);
                    false
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.error_scroll = self.error_scroll.saturating_sub(1);
                    false
                }
                _ => false,
            };
        }

        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Char('l') => {
                self.screen = Screen::Logs;
                self.auto_scroll = true;
                false
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.unresolved_error.is_some() {
                    self.error_expanded = true;
                    self.error_scroll = 0;
                }
                false
            }
            _ => false,
        }
    }
}

fn level_color(level: &Level) -> Color {
    match *level {
        Level::ERROR => Color::Red,
        Level::WARN => Color::Yellow,
        Level::INFO => Color::Green,
        Level::DEBUG | Level::TRACE => Color::DarkGray,
    }
}

fn level_symbol(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "✕",
        Level::WARN => "▲",
        Level::INFO => "●",
        Level::DEBUG | Level::TRACE => "·",
    }
}

/// Run the TUI event loop. Blocks until the user quits.
pub async fn run_tui(log_buffer: LogBuffer, startup_info: StartupInfo) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        original_hook(info);
    }));

    let mut app = TuiApp::new(log_buffer, startup_info);
    let mut event_stream = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    loop {
        app.tick();
        terminal.draw(|f| app.render(f))?;

        tokio::select! {
            _ = tick.tick() => {}
            event = event_stream.next() => {
                match event {
                    Some(Ok(ev)) => {
                        if app.handle_event(ev) {
                            break;
                        }
                    }
                    Some(Err(_)) => break,
                    None => break,
                }
            }
        }
    }

    ratatui::restore();
    Ok(())
}
