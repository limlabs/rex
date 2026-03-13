pub mod log_layer;
mod mascot;

use crate::tui::log_layer::{LogBuffer, LogEntry};
use crate::tui::mascot::{render_mascot, MascotState};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
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
    screen: Screen,
    log_buffer: LogBuffer,
    startup_info: StartupInfo,
    log_filter: LogFilter,
    log_scroll: usize,
    search_query: String,
    searching: bool,
    auto_scroll: bool,
    /// Countdown ticks for the tail-flick animation on HMR rebuild.
    tail_flick_ticks: u8,
    /// Last log message we saw, used to detect new rebuild events.
    last_seen_msg: String,
    /// Currently unresolved error. Set on ERROR log, cleared on successful rebuild.
    unresolved_error: Option<UnresolvedError>,
}

impl TuiApp {
    fn new(log_buffer: LogBuffer, startup_info: StartupInfo) -> Self {
        let last_seen_msg = log_buffer.last().map(|e| e.message).unwrap_or_default();
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
            last_seen_msg,
            unresolved_error: None,
        }
    }

    /// Advance animation state and detect HMR rebuilds.
    fn tick(&mut self) {
        if let Some(entry) = self.log_buffer.last() {
            if entry.message != self.last_seen_msg {
                if entry.message.starts_with("Rebuild complete") {
                    self.tail_flick_ticks = 4;
                    self.unresolved_error = None;
                } else if entry.level == Level::ERROR {
                    self.unresolved_error = Some(UnresolvedError {
                        message: entry.message.clone(),
                    });
                }
                self.last_seen_msg = entry.message;
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

    fn render(&self, f: &mut Frame<'_>) {
        match self.screen {
            Screen::Home => self.render_home(f),
            Screen::Logs => self.render_logs(f),
        }
    }

    fn render_home(&self, f: &mut Frame<'_>) {
        let area = f.area();

        let cw = area.width.saturating_sub(4).max(1) as usize;
        let error_lines = self
            .unresolved_error
            .as_ref()
            .map(|e| {
                let wrapped: usize = e
                    .message
                    .lines()
                    .take(12)
                    .map(|l| l.chars().count().max(1).div_ceil(cw))
                    .sum();
                wrapped as u16 + 2
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
            Self::render_error_section(f, chunks[2], err, area.width);
        }
        self.render_home_footer(f, chunks[4]);
    }

    fn render_mascot_and_info(&self, f: &mut Frame<'_>, area: Rect) {
        let cols = Layout::horizontal([Constraint::Length(25), Constraint::Min(0)]).split(area);

        render_mascot(f, cols[0], self.mascot_state());
        self.render_info(f, cols[1]);
    }

    fn render_error_section(f: &mut Frame<'_>, area: Rect, err: &UnresolvedError, _w: u16) {
        let mut lines: Vec<Line<'_>> = Vec::new();

        // Header
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "✕ Error",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ]));

        // Error message lines
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
                    "  ... press l for full logs",
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
            route_parts.push(format!(
                "{} {}",
                info.page_count,
                if info.page_count == 1 {
                    "page"
                } else {
                    "pages"
                }
            ));
        }
        if info.api_count > 0 {
            route_parts.push(format!(
                "{} API {}",
                info.api_count,
                if info.api_count == 1 {
                    "route"
                } else {
                    "routes"
                }
            ));
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
            .map(|feat| format!("◇ {feat}"))
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

        // Blank line then last log — but omit if it duplicates the unresolved error
        // (the error is already shown in the error section below).
        lines.push(Line::from(""));
        let last_entry = self.log_buffer.last();
        let is_duplicate_error = match (&last_entry, &self.unresolved_error) {
            (Some(entry), Some(err)) => entry.message == err.message,
            _ => false,
        };
        let last_log_line = if is_duplicate_error {
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
                None => Line::from(vec![Span::styled(
                    "Waiting for activity...",
                    Style::default().fg(Color::DarkGray),
                )]),
            }
        };
        lines.push(last_log_line);

        let para = Paragraph::new(lines);
        f.render_widget(para, area);
    }

    fn render_home_footer(&self, f: &mut Frame<'_>, area: Rect) {
        let footer = Line::from(vec![
            Span::raw("  "),
            Span::styled("l", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" Logs", Style::default().fg(Color::DarkGray)),
            Span::styled("  ·  ", Style::default().fg(Color::DarkGray)),
            Span::styled("q", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
        ]);
        let para = Paragraph::new(footer);
        f.render_widget(para, area);
    }

    fn render_logs(&self, f: &mut Frame<'_>) {
        let area = f.area();

        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

        self.render_log_filter_bar(f, chunks[0]);
        self.render_log_list(f, chunks[1]);

        if self.searching {
            self.render_search_bar(f, chunks[2]);
        } else {
            self.render_log_footer(f, chunks[2]);
        }
    }

    fn render_log_filter_bar(&self, f: &mut Frame<'_>, area: Rect) {
        let filters = [
            (LogFilter::All, "a"),
            (LogFilter::Error, "e"),
            (LogFilter::Warn, "w"),
            (LogFilter::Info, "i"),
            (LogFilter::Debug, "d"),
        ];

        let mut spans = vec![Span::raw(" ")];
        for (i, (filter, key)) in filters.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", Style::default().fg(Color::DarkGray)));
            }
            let is_active = self.log_filter == *filter;
            let style = if is_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(EMERALD)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!(" [{key}]{} ", filter.label()), style));
        }

        if !self.search_query.is_empty() {
            spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("search: \"{}\"", self.search_query),
                Style::default().fg(Color::Yellow),
            ));
        }

        let para = Paragraph::new(Line::from(spans));
        f.render_widget(para, area);
    }

    fn render_log_list(&self, f: &mut Frame<'_>, area: Rect) {
        let entries = self.filtered_entries();
        let visible_height = area.height as usize;

        let scroll = if self.auto_scroll {
            entries.len().saturating_sub(visible_height)
        } else {
            self.log_scroll
                .min(entries.len().saturating_sub(visible_height))
        };

        let items: Vec<ListItem<'_>> = entries
            .iter()
            .skip(scroll)
            .take(visible_height)
            .map(|entry| {
                let color = level_color(&entry.level);
                let mut spans = vec![Span::styled(
                    format!(" {} ", level_symbol(&entry.level)),
                    Style::default().fg(color),
                )];

                if !self.search_query.is_empty() {
                    let msg = &entry.message;
                    let query_lower = self.search_query.to_lowercase();
                    let msg_lower = msg.to_lowercase();
                    let mut last_end = 0;
                    let mut pos = 0;

                    while let Some(idx) = msg_lower[pos..].find(&query_lower) {
                        let start = pos + idx;
                        let end = start + self.search_query.len();
                        if start > last_end {
                            spans.push(Span::raw(msg[last_end..start].to_string()));
                        }
                        spans.push(Span::styled(
                            msg[start..end].to_string(),
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ));
                        last_end = end;
                        pos = end;
                    }
                    if last_end < msg.len() {
                        spans.push(Span::raw(msg[last_end..].to_string()));
                    }
                } else {
                    spans.push(Span::raw(entry.message.clone()));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::NONE));
        f.render_widget(list, area);
    }

    fn render_search_bar(&self, f: &mut Frame<'_>, area: Rect) {
        let line = Line::from(vec![
            Span::styled(
                " /",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(&self.search_query),
            Span::styled("█", Style::default().fg(Color::DarkGray)),
        ]);
        let para = Paragraph::new(line);
        f.render_widget(para, area);
    }

    fn render_log_footer(&self, f: &mut Frame<'_>, area: Rect) {
        let entries = self.filtered_entries();
        let count = entries.len();
        let footer = Line::from(vec![
            Span::raw(" "),
            Span::styled("j/k", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" scroll", Style::default().fg(Color::DarkGray)),
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::styled("g/G", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" top/bottom", Style::default().fg(Color::DarkGray)),
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::styled("/", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" search", Style::default().fg(Color::DarkGray)),
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" back", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("  {count} entries"),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        let para = Paragraph::new(footer);
        f.render_widget(para, area);
    }

    fn filtered_entries(&self) -> Vec<LogEntry> {
        let snapshot = self.log_buffer.snapshot();
        let query_lower = self.search_query.to_lowercase();

        snapshot
            .into_iter()
            .filter(|e| self.log_filter.matches(&e.level))
            .filter(|e| query_lower.is_empty() || e.message.to_lowercase().contains(&query_lower))
            .collect()
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
        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Char('l') => {
                self.screen = Screen::Logs;
                self.auto_scroll = true;
                false
            }
            _ => false,
        }
    }

    fn handle_logs_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Esc => {
                self.screen = Screen::Home;
                false
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.auto_scroll = false;
                self.log_scroll = self.log_scroll.saturating_add(1);
                false
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.auto_scroll = false;
                self.log_scroll = self.log_scroll.saturating_sub(1);
                false
            }
            KeyCode::Char('G') => {
                self.auto_scroll = true;
                false
            }
            KeyCode::Char('g') => {
                self.auto_scroll = false;
                self.log_scroll = 0;
                false
            }
            KeyCode::Char('/') => {
                self.searching = true;
                self.search_query.clear();
                false
            }
            KeyCode::Char('n') => {
                if !self.search_query.is_empty() {
                    self.auto_scroll = false;
                    self.log_scroll = self.log_scroll.saturating_add(1);
                }
                false
            }
            KeyCode::Char('N') => {
                if !self.search_query.is_empty() {
                    self.auto_scroll = false;
                    self.log_scroll = self.log_scroll.saturating_sub(1);
                }
                false
            }
            KeyCode::Char('e') => {
                self.log_filter = LogFilter::Error;
                false
            }
            KeyCode::Char('w') => {
                self.log_filter = LogFilter::Warn;
                false
            }
            KeyCode::Char('i') => {
                self.log_filter = LogFilter::Info;
                false
            }
            KeyCode::Char('a') => {
                self.log_filter = LogFilter::All;
                false
            }
            KeyCode::Char('d') => {
                self.log_filter = LogFilter::Debug;
                false
            }
            _ => false,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.searching = false;
                false
            }
            KeyCode::Enter => {
                self.searching = false;
                false
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                false
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
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
        Level::DEBUG => Color::DarkGray,
        Level::TRACE => Color::DarkGray,
    }
}

fn level_symbol(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "✕",
        Level::WARN => "▲",
        Level::INFO => "●",
        Level::DEBUG => "·",
        Level::TRACE => "·",
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
