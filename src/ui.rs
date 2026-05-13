use ratatui::prelude::*;
use ratatui::widgets::{Bar, BarChart, BarGroup, Block, Padding, Paragraph, Row, Table};

use crate::data::{self, Metrics};

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChartView {
    Tokens,
    Cost,
}

pub struct App {
    pub metrics: Metrics,
    /// Flattened daily data for the chart
    pub daily_vec: Vec<(chrono::NaiveDate, crate::data::DayTokens)>,
    /// Per-day cost in USD, aligned with daily_vec by date.
    pub daily_cost_vec: Vec<(chrono::NaiveDate, f64)>,
    pub scroll_offset: usize,
    pub chart_view: ChartView,
}

impl App {
    pub fn new(metrics: Metrics) -> Self {
        let daily_vec: Vec<_> = metrics
            .daily_tokens
            .iter()
            .map(|(d, t)| (*d, t.clone()))
            .collect();
        let daily_cost_vec: Vec<_> = daily_vec
            .iter()
            .map(|(d, _)| {
                let cost = metrics
                    .daily_model_tokens
                    .get(d)
                    .map(data::estimate_daily_cost)
                    .unwrap_or(0.0);
                (*d, cost)
            })
            .collect();
        Self {
            metrics,
            daily_vec,
            daily_cost_vec,
            scroll_offset: 0,
            chart_view: ChartView::Tokens,
        }
    }

    pub fn toggle_chart_view(&mut self) {
        self.chart_view = match self.chart_view {
            ChartView::Tokens => ChartView::Cost,
            ChartView::Cost => ChartView::Tokens,
        };
    }

    /// Compute how many bars fit in the chart area.
    /// The chart occupies 70% of terminal width (top-left panel), minus 2 for borders.
    pub fn visible_bars_for_terminal(&self, terminal_width: u16) -> usize {
        // The chart is 70% of the terminal width
        let chart_area_width = (terminal_width as u32 * 70 / 100) as u16;
        self.visible_bars(chart_area_width)
    }

    pub fn visible_bars(&self, chart_width: u16) -> usize {
        let w = chart_width.saturating_sub(2) as usize;
        (w / 8).max(1)
    }

    pub fn scroll_right(&mut self, visible: usize) {
        let max = self.daily_vec.len().saturating_sub(visible);
        self.scroll_offset = (self.scroll_offset + 3).min(max);
    }

    pub fn scroll_left(&mut self, _visible: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(3);
    }

    pub fn scroll_to_end(&mut self, visible: usize) {
        self.scroll_offset = self.daily_vec.len().saturating_sub(visible);
    }
}

// ---------------------------------------------------------------------------
// Main render
// ---------------------------------------------------------------------------

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Top-level: header(3), body(fill), footer(3)
    let outer = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Length(3),
    ])
    .split(area);

    render_header(frame, outer[0]);
    render_body(frame, outer[1], app);
    render_footer(frame, outer[2], app);
}

fn render_header(frame: &mut Frame, area: Rect) {
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " metrix ",
            Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
        ),
        Span::raw("  Claude Code usage metrics"),
    ]))
    .block(Block::bordered().padding(Padding::horizontal(1)));
    frame.render_widget(title, area);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let total_tokens: u64 = app.daily_vec.iter().map(|(_, d)| d.total()).sum();
    let total_days = app.daily_vec.len();
    let total_sessions = app.metrics.sessions.len();
    let total_cost: f64 = app
        .metrics
        .model_tokens
        .iter()
        .map(|(model, tokens)| data::estimate_cost(model, tokens))
        .sum();
    let footer_text = format!(
        " {} days | {} sessions | {:.1}M tokens | ~${:.2} est. cost | ←/→ scroll | q quit",
        total_days,
        total_sessions,
        total_tokens as f64 / 1_000_000.0,
        total_cost,
    );
    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::bordered().padding(Padding::horizontal(1)));
    frame.render_widget(footer, area);
}

// ---------------------------------------------------------------------------
// Body layout
// ---------------------------------------------------------------------------

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    // Split body into top row (chart) and bottom row (panels)
    let rows =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    // Top row: token chart (left 70%) + heatmap (right 30%)
    let top_cols =
        Layout::horizontal([Constraint::Percentage(70), Constraint::Percentage(30)]).split(rows[0]);

    render_token_chart(frame, top_cols[0], app);
    render_heatmap(frame, top_cols[1], app);

    // Bottom row: 5 panels
    let bot_cols = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
    ])
    .split(rows[1]);

    render_tool_calls(frame, bot_cols[0], app);
    render_file_ops(frame, bot_cols[1], app);
    render_session_records(frame, bot_cols[2], app);
    render_conversations(frame, bot_cols[3], app);
    render_cost(frame, bot_cols[4], app);
}

// ---------------------------------------------------------------------------
// Token chart (bar chart)
// ---------------------------------------------------------------------------

fn render_token_chart(frame: &mut Frame, area: Rect, app: &App) {
    let (base_title, bar_color) = match app.chart_view {
        ChartView::Tokens => ("Tokens per Day", Color::Cyan),
        ChartView::Cost => ("Cost per Day", Color::Green),
    };

    if app.daily_vec.is_empty() {
        let msg = Paragraph::new("No data found in ~/.claude/projects/")
            .style(Style::default().fg(Color::Red))
            .block(Block::bordered().title(format!(" {} ", base_title)));
        frame.render_widget(msg, area);
        return;
    }

    let visible = app.visible_bars(area.width);
    let start = app.scroll_offset;
    let end = (start + visible).min(app.daily_vec.len());

    let bars: Vec<Bar> = (start..end)
        .map(|i| {
            let (date, tokens) = &app.daily_vec[i];
            let label = date.format("%b %d").to_string();
            match app.chart_view {
                ChartView::Tokens => {
                    let total = tokens.total();
                    Bar::default()
                        .value(total)
                        .label(Line::from(label))
                        .text_value(format_tokens_short(total))
                        .style(Style::default().fg(bar_color))
                }
                ChartView::Cost => {
                    let cost = app.daily_cost_vec[i].1;
                    // BarChart needs an integer value; render cost in cents.
                    let cents = (cost * 100.0).round().max(0.0) as u64;
                    Bar::default()
                        .value(cents)
                        .label(Line::from(label))
                        .text_value(format_cost_short(cost))
                        .style(Style::default().fg(bar_color))
                }
            }
        })
        .collect();

    let title = if app.daily_vec.len() > visible {
        format!(
            " {} [{}-{}/{}]  (c: toggle) ",
            base_title,
            start + 1,
            end,
            app.daily_vec.len()
        )
    } else {
        format!(" {}  (c: toggle) ", base_title)
    };

    let chart = BarChart::default()
        .block(Block::bordered().title(title))
        .data(BarGroup::default().bars(&bars))
        .bar_width(7)
        .bar_gap(1)
        .bar_style(Style::default().fg(bar_color))
        .value_style(Style::default().fg(Color::Black).bg(bar_color).bold());

    frame.render_widget(chart, area);
}

// ---------------------------------------------------------------------------
// Hour-of-day heatmap
// ---------------------------------------------------------------------------

fn render_heatmap(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Activity by Hour ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let max_count = app
        .metrics
        .hour_counts
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);

    // Render as a vertical list: "09:00  ████████  1234"
    let lines: Vec<Line> = (0..24)
        .filter_map(|h| {
            if h as u16 >= inner.height {
                return None;
            }
            let count = app.metrics.hour_counts[h];
            let bar_max_width = inner.width.saturating_sub(14) as usize; // "HH:00  " (7) + "  NNNNN" (7)
            let bar_len = if max_count > 0 {
                ((count as f64 / max_count as f64) * bar_max_width as f64).ceil() as usize
            } else {
                0
            };
            let intensity = if max_count > 0 {
                count as f64 / max_count as f64
            } else {
                0.0
            };
            let bar_str: String = "█".repeat(bar_len);
            let pad: String = " ".repeat(bar_max_width.saturating_sub(bar_len));
            // Red/orange gradient: low=dark red, high=bright orange
            let r = (80.0 + intensity * 175.0) as u8;
            let g = (intensity * 140.0) as u8;
            let b = 0u8;
            let color = if count == 0 {
                Color::Rgb(40, 20, 0)
            } else {
                Color::Rgb(r, g, b)
            };
            Some(Line::from(vec![
                Span::styled(
                    format!("{:02}:00 ", h),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(bar_str, Style::default().fg(color)),
                Span::raw(pad),
                Span::styled(
                    format!(" {:>5}", count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
        })
        .collect();

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

// ---------------------------------------------------------------------------
// Tool calls breakdown
// ---------------------------------------------------------------------------

fn render_tool_calls(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Tool Calls ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Sort by count desc
    let mut tools: Vec<_> = app.metrics.tool_calls.iter().collect();
    tools.sort_by(|a, b| b.1.cmp(a.1));

    let total: u64 = tools.iter().map(|(_, c)| **c).sum();

    let rows: Vec<Row> = tools
        .iter()
        .take(inner.height.saturating_sub(1) as usize)
        .map(|(name, count)| {
            let pct = if total > 0 {
                (**count as f64 / total as f64 * 100.0) as u16
            } else {
                0
            };
            let bar_w = inner.width.saturating_sub(20) as usize;
            let filled = (pct as usize * bar_w / 100).max(if **count > 0 { 1 } else { 0 });
            let bar: String = "█".repeat(filled);
            Row::new(vec![
                format!("{:<10}", name),
                format!("{:>5}", count),
                format!(" {}", bar),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Fill(1),
        ],
    )
    .style(Style::default().fg(Color::White))
    .column_spacing(1);

    frame.render_widget(table, inner);
}

// ---------------------------------------------------------------------------
// File ops panel
// ---------------------------------------------------------------------------

fn render_file_ops(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" File Operations ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let ops = &app.metrics.file_ops;

    // Top: summary counts
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Created  ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{:>6}", ops.created),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Edited   ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{:>6}", ops.edited),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Read     ", Style::default().fg(Color::Blue)),
            Span::styled(
                format!("{:>6}", ops.read),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Most Edited Files",
            Style::default().fg(Color::DarkGray).bold(),
        )),
    ];

    // Top edited files
    let mut edits: Vec<_> = app.metrics.file_edit_counts.iter().collect();
    edits.sort_by(|a, b| b.1.cmp(a.1));
    let max_rows = inner.height.saturating_sub(6) as usize;
    for (path, count) in edits.iter().take(max_rows) {
        let display = truncate_left(path, inner.width.saturating_sub(7) as usize);
        lines.push(Line::from(vec![
            Span::styled(format!("{:>5} ", count), Style::default().fg(Color::Yellow)),
            Span::styled(display, Style::default().fg(Color::DarkGray)),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

// ---------------------------------------------------------------------------
// Session records panel
// ---------------------------------------------------------------------------

fn render_session_records(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Session Records ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sessions = &app.metrics.sessions;
    if sessions.is_empty() {
        let p = Paragraph::new("No session data").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, inner);
        return;
    }

    // Longest session
    let longest = &sessions[0]; // already sorted desc by duration

    // Most files touched
    let most_files = sessions.iter().max_by_key(|s| s.files_touched_count);

    // Most turns
    let most_turns = sessions
        .iter()
        .max_by_key(|s| s.user_turns + s.assistant_turns);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "Longest Session",
            Style::default().fg(Color::Magenta).bold(),
        )),
        Line::from(vec![
            Span::styled("  Duration  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_duration(longest.duration_secs),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Date      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_session_date(longest),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Turns     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", longest.user_turns + longest.assistant_turns),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
    ];

    if let Some(mf) = most_files {
        lines.push(Line::from(Span::styled(
            "Most Files Touched",
            Style::default().fg(Color::Green).bold(),
        )));
        lines.push(Line::from(vec![
            Span::styled("  Files     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", mf.files_touched_count),
                Style::default().fg(Color::White).bold(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Date      ", Style::default().fg(Color::DarkGray)),
            Span::styled(format_session_date(mf), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(""));
    }

    if let Some(mt) = most_turns {
        lines.push(Line::from(Span::styled(
            "Most Turns",
            Style::default().fg(Color::Blue).bold(),
        )));
        lines.push(Line::from(vec![
            Span::styled("  Turns     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", mt.user_turns + mt.assistant_turns),
                Style::default().fg(Color::White).bold(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Date      ", Style::default().fg(Color::DarkGray)),
            Span::styled(format_session_date(mt), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Duration  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_duration(mt.duration_secs),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

// ---------------------------------------------------------------------------
// Conversation turns panel
// ---------------------------------------------------------------------------

fn render_conversations(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Conversations ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let m = &app.metrics;
    let total_turns = m.total_user_turns + m.total_assistant_turns;
    let avg_turns_per_session = if !m.sessions.is_empty() {
        total_turns as f64 / m.sessions.len() as f64
    } else {
        0.0
    };

    // Turn distribution: bucket sessions by turn count
    let mut turn_buckets: Vec<(&str, usize)> = vec![
        ("1-5", 0),
        ("6-20", 0),
        ("21-50", 0),
        ("51-100", 0),
        ("100+", 0),
    ];
    for s in &m.sessions {
        let t = (s.user_turns + s.assistant_turns) as usize;
        if t <= 5 {
            turn_buckets[0].1 += 1;
        } else if t <= 20 {
            turn_buckets[1].1 += 1;
        } else if t <= 50 {
            turn_buckets[2].1 += 1;
        } else if t <= 100 {
            turn_buckets[3].1 += 1;
        } else {
            turn_buckets[4].1 += 1;
        }
    }
    let bucket_max = turn_buckets
        .iter()
        .map(|(_, c)| *c)
        .max()
        .unwrap_or(1)
        .max(1);

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Total Turns    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", total_turns),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  User         ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", m.total_user_turns),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Assistant    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", m.total_assistant_turns),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Line::from(vec![
            Span::styled("Avg/Session    ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}", avg_turns_per_session),
                Style::default().fg(Color::White).bold(),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Session Size Distribution",
            Style::default().fg(Color::DarkGray).bold(),
        )),
    ];

    // Label is 14 chars ("{:>7} turns "), count suffix is 5 (" NNNN")
    let bar_max_w = (inner.width as usize).saturating_sub(19);
    for (label, count) in &turn_buckets {
        let filled = if bar_max_w > 0 {
            (*count as f64 / bucket_max as f64 * bar_max_w as f64).ceil() as usize
        } else {
            0
        };
        let filled = filled.min(bar_max_w).max(if *count > 0 { 1 } else { 0 });
        let bar: String = "█".repeat(filled);
        let pad: String = " ".repeat(bar_max_w.saturating_sub(filled));
        let count_str = format!("{:>4}", count);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>7} turns ", label),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(bar, Style::default().fg(Color::Cyan)),
            Span::raw(pad),
            Span::styled(
                format!(" {}", count_str),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

// ---------------------------------------------------------------------------
// Cost estimation panel
// ---------------------------------------------------------------------------

fn render_cost(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().title(" Cost Estimate ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let m = &app.metrics;
    let mut lines: Vec<Line> = Vec::new();

    // Total cost
    let total_cost: f64 = m
        .model_tokens
        .iter()
        .map(|(model, tokens)| data::estimate_cost(model, tokens))
        .sum();

    lines.push(Line::from(vec![
        Span::styled("Total        ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("${:.2}", total_cost),
            Style::default().fg(Color::Green).bold(),
        ),
    ]));

    // Daily cost stats: average across active days, plus last-7-days sum.
    let active_days = app.daily_cost_vec.iter().filter(|(_, c)| *c > 0.0).count();
    let daily_avg = if active_days > 0 {
        total_cost / active_days as f64
    } else {
        0.0
    };
    lines.push(Line::from(vec![
        Span::styled("Daily avg    ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("${:.2}", daily_avg),
            Style::default().fg(Color::White).bold(),
        ),
    ]));

    let last_7_total: f64 = if let Some((most_recent, _)) = app.daily_cost_vec.last() {
        let cutoff = *most_recent - chrono::Duration::days(6);
        app.daily_cost_vec
            .iter()
            .rev()
            .take_while(|(d, _)| *d >= cutoff)
            .map(|(_, c)| *c)
            .sum()
    } else {
        0.0
    };
    lines.push(Line::from(vec![
        Span::styled("Last 7 days  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("${:.2}", last_7_total),
            Style::default().fg(Color::White).bold(),
        ),
    ]));
    lines.push(Line::from(""));

    // Per-model breakdown
    lines.push(Line::from(Span::styled(
        "By Model",
        Style::default().fg(Color::DarkGray).bold(),
    )));

    let mut model_costs: Vec<_> = m
        .model_tokens
        .iter()
        .map(|(model, tokens)| {
            let cost = data::estimate_cost(model, tokens);
            (model.clone(), tokens.clone(), cost)
        })
        .collect();
    model_costs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    for (model, _tokens, cost) in &model_costs {
        if *cost < 0.01 {
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<11}", model), Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("${:.2}", cost),
                Style::default().fg(Color::White).bold(),
            ),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "By Category",
        Style::default().fg(Color::DarkGray).bold(),
    )));

    // Aggregate costs by category across all models
    let mut total_input_cost = 0.0f64;
    let mut total_output_cost = 0.0f64;
    let mut total_cache_write_cost = 0.0f64;
    let mut total_cache_read_cost = 0.0f64;
    for (model, tokens) in &m.model_tokens {
        let (inp, outp, cw, cr) = model_prices(model);
        let mtok = 1_000_000.0;
        total_input_cost += tokens.input as f64 / mtok * inp;
        total_output_cost += tokens.output as f64 / mtok * outp;
        total_cache_write_cost += tokens.cache_creation as f64 / mtok * cw;
        total_cache_read_cost += tokens.cache_read as f64 / mtok * cr;
    }

    let categories = [
        ("  Cache write", total_cache_write_cost),
        ("  Cache read ", total_cache_read_cost),
        ("  Output     ", total_output_cost),
        ("  Input      ", total_input_cost),
    ];
    for (label, cost) in &categories {
        lines.push(Line::from(vec![
            Span::styled(*label, Style::default().fg(Color::DarkGray)),
            Span::styled(format!("  ${:.2}", cost), Style::default().fg(Color::White)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Based on API pricing",
        Style::default().fg(Color::Rgb(80, 80, 80)),
    )));

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn model_prices(model: &str) -> (f64, f64, f64, f64) {
    // (input_per_mtok, output_per_mtok, cache_write_per_mtok, cache_read_per_mtok)
    match model {
        "Opus 4.6" | "Opus 4.5" => (5.0, 25.0, 6.25, 0.50),
        "Sonnet 4.5" => (3.0, 15.0, 3.75, 0.30),
        "Haiku 4.5" => (1.0, 5.0, 1.25, 0.10),
        _ => (5.0, 25.0, 6.25, 0.50),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_cost_short(cost: f64) -> String {
    if cost >= 100.0 {
        format!("${:.0}", cost)
    } else if cost >= 1.0 {
        format!("${:.1}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

fn format_tokens_short(total: u64) -> String {
    if total >= 100_000_000 {
        format!("{:.0}M", total as f64 / 1_000_000.0)
    } else if total >= 1_000_000 {
        format!("{:.1}M", total as f64 / 1_000_000.0)
    } else if total >= 1_000 {
        format!("{:.0}K", total as f64 / 1_000.0)
    } else {
        total.to_string()
    }
}

fn format_duration(secs: u64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

fn format_session_date(session: &data::SessionStats) -> String {
    match session.first_date {
        Some(d) => d.format("%b %d, %Y").to_string(),
        None => "unknown".to_string(),
    }
}

/// Truncate from the left, showing "…end/of/path"
fn truncate_left(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let start = s.len() - max_len.saturating_sub(1);
        format!("…{}", &s[start..])
    }
}
