use super::log_layer::LogEntry;
use super::text::{highlight_search, truncate_str, wrap_message};
use super::{level_color, level_symbol, LogFilter, Screen, TuiApp, EMERALD};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

impl TuiApp {
    pub(super) fn render_logs(&mut self, f: &mut Frame<'_>) {
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
            Self::render_search_bar(f, chunks[2], &self.search_query);
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
            let style = if self.log_filter == *filter {
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
        f.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_log_list(&mut self, f: &mut Frame<'_>, area: Rect) {
        let entries = self.filtered_entries();
        if entries.is_empty() {
            return;
        }

        let visible_height = area.height as usize;
        // Prefix: " HH:MM:SS ● " = 12 chars
        let prefix_width = 12;
        let msg_width = (area.width as usize).saturating_sub(prefix_width);
        let entry_count = entries.len();

        // Clamp cursor
        let cursor = if self.auto_scroll {
            entry_count.saturating_sub(1)
        } else {
            self.log_cursor.min(entry_count.saturating_sub(1))
        };
        self.log_cursor = cursor;

        // Build all visual lines
        let mut all_lines: Vec<Line<'_>> = Vec::new();
        let mut cursor_start = 0usize;

        for (i, entry) in entries.iter().enumerate() {
            let color = level_color(&entry.level);
            let is_selected = i == cursor;
            let visual = wrap_message(&entry.message, msg_width);
            let overflow = visual.len().saturating_sub(1);
            let is_expanded = self.expanded_entries.contains(&i);

            if is_selected {
                cursor_start = all_lines.len();
            }

            let sel_bg = if is_selected {
                Style::default().bg(Color::Rgb(35, 35, 50))
            } else {
                Style::default()
            };
            let time_prefix = format!(" {} ", entry.timestamp);
            let sym_prefix = format!("{} ", level_symbol(&entry.level));
            let indent = " ".repeat(prefix_width);

            if is_expanded || overflow == 0 {
                // Show all lines
                for (j, segment) in visual.iter().enumerate() {
                    let mut spans = Vec::new();
                    if j == 0 {
                        spans.push(Span::styled(
                            time_prefix.clone(),
                            Style::default().fg(Color::DarkGray),
                        ));
                        spans.push(Span::styled(sym_prefix.clone(), Style::default().fg(color)));
                    } else {
                        spans.push(Span::raw(indent.clone()));
                    }
                    if !self.search_query.is_empty() {
                        highlight_search(&mut spans, segment, &self.search_query);
                    } else {
                        spans.push(Span::raw(segment.clone()));
                    }
                    all_lines.push(Line::from(spans).style(sel_bg));
                }
            } else {
                // Collapsed: first line + overflow indicator
                let indicator = format!(" …+{overflow}");
                let max_text = msg_width.saturating_sub(indicator.len());
                let display = truncate_str(&visual[0], max_text);
                let mut spans = vec![
                    Span::styled(time_prefix.clone(), Style::default().fg(Color::DarkGray)),
                    Span::styled(sym_prefix.clone(), Style::default().fg(color)),
                ];
                if !self.search_query.is_empty() {
                    highlight_search(&mut spans, display, &self.search_query);
                } else {
                    spans.push(Span::raw(display.to_string()));
                }
                spans.push(Span::styled(
                    indicator,
                    Style::default().fg(Color::DarkGray),
                ));
                all_lines.push(Line::from(spans).style(sel_bg));
            }
        }

        // Scroll: keep cursor visible
        let total = all_lines.len();
        let scroll = if self.auto_scroll {
            total.saturating_sub(visible_height)
        } else {
            let cur = self.log_scroll;
            if cursor_start < cur {
                cursor_start
            } else if cursor_start >= cur + visible_height {
                cursor_start.saturating_sub(visible_height / 3)
            } else {
                cur.min(total.saturating_sub(visible_height))
            }
        };
        self.log_scroll = scroll;

        f.render_widget(Paragraph::new(all_lines).scroll((scroll as u16, 0)), area);
    }

    fn render_search_bar(f: &mut Frame<'_>, area: Rect, query: &str) {
        let line = Line::from(vec![
            Span::styled(
                " /",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(query),
            Span::styled("█", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn render_log_footer(&self, f: &mut Frame<'_>, area: Rect) {
        let count = self.filtered_entries().len();
        let footer = Line::from(vec![
            Span::raw(" "),
            Span::styled("j/k", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" navigate", Style::default().fg(Color::DarkGray)),
            Span::styled("  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Space", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(" expand", Style::default().fg(Color::DarkGray)),
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
        f.render_widget(Paragraph::new(footer), area);
    }

    pub(super) fn filtered_entries(&self) -> Vec<LogEntry> {
        let snapshot = self.log_buffer.snapshot();
        let query_lower = self.search_query.to_lowercase();
        snapshot
            .into_iter()
            .filter(|e| self.log_filter.matches(&e.level))
            .filter(|e| query_lower.is_empty() || e.message.to_lowercase().contains(&query_lower))
            .collect()
    }

    pub(super) fn handle_logs_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => true,
            KeyCode::Esc => {
                self.screen = Screen::Home;
                false
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.auto_scroll = false;
                self.log_cursor = self.log_cursor.saturating_add(1);
                false
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.auto_scroll = false;
                self.log_cursor = self.log_cursor.saturating_sub(1);
                false
            }
            KeyCode::Char(' ') => {
                let idx = self.log_cursor;
                if !self.expanded_entries.remove(&idx) {
                    self.expanded_entries.insert(idx);
                }
                false
            }
            KeyCode::Char('G') => {
                self.auto_scroll = true;
                false
            }
            KeyCode::Char('g') => {
                self.auto_scroll = false;
                self.log_cursor = 0;
                false
            }
            KeyCode::Char('/') => {
                self.searching = true;
                self.search_query.clear();
                self.expanded_entries.clear();
                false
            }
            KeyCode::Char('n') => {
                if !self.search_query.is_empty() {
                    self.auto_scroll = false;
                    self.log_cursor = self.log_cursor.saturating_add(1);
                }
                false
            }
            KeyCode::Char('N') => {
                if !self.search_query.is_empty() {
                    self.auto_scroll = false;
                    self.log_cursor = self.log_cursor.saturating_sub(1);
                }
                false
            }
            KeyCode::Char(c @ ('e' | 'w' | 'i' | 'a' | 'd')) => {
                self.log_filter = match c {
                    'e' => LogFilter::Error,
                    'w' => LogFilter::Warn,
                    'i' => LogFilter::Info,
                    'a' => LogFilter::All,
                    _ => LogFilter::Debug,
                };
                self.expanded_entries.clear();
                self.log_cursor = 0;
                false
            }
            _ => false,
        }
    }

    pub(super) fn handle_search_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.searching = false;
                self.expanded_entries.clear();
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
