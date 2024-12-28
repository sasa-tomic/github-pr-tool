use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Text},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Tabs},
    Terminal,
};
use std::io;

pub struct TabsState<'a> {
    pub titles: Vec<&'a str>,
    pub index: usize,
}

impl<'a> TabsState<'a> {
    pub fn new(titles: Vec<&'a str>) -> Self {
        Self { titles, index: 0 }
    }
    pub fn next(&mut self) {
        self.index = (self.index + 1) % self.titles.len();
    }
    pub fn previous(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        } else {
            self.index = self.titles.len() - 1;
        }
    }
}

pub struct App<'a> {
    pub title: &'a str,
    pub should_quit: bool,
    pub tabs: TabsState<'a>,
    pub logs: Vec<(&'a str, String)>,
    pub errors: Vec<String>,
    pub progress: f64,
    pub details: String,
    pub error_tab_blink: bool,
    pub blink_timer: u8,
}

impl<'a> App<'a> {
    pub fn new(title: &'a str) -> Self {
        App {
            title,
            should_quit: false,
            tabs: TabsState::new(vec!["Logs", "Errors", "Details", "Status"]),
            logs: vec![],
            errors: vec![],
            progress: 0.0,
            details: String::new(),
            error_tab_blink: false,
            blink_timer: 0,
        }
    }

    pub fn on_right(&mut self) {
        self.tabs.next();
    }

    pub fn on_left(&mut self) {
        self.tabs.previous();
    }

    pub fn switch_to_tab(&mut self, index: usize) {
        self.tabs.index = index;
    }

    pub fn update_progress(&mut self, value: f64) {
        self.progress = value;
    }

    pub fn add_log<S: ToString>(&mut self, level: &'a str, message: S) {
        self.logs.push((level, message.to_string()));
    }

    pub fn add_error<S: ToString>(&mut self, error: S) {
        self.errors.push(error.to_string());
        self.start_error_blink();
    }

    pub fn update_details(&mut self, details: String) {
        self.details = details;
    }

    pub fn start_error_blink(&mut self) {
        self.error_tab_blink = true;
        self.blink_timer = 10; // Will blink 5 times (10 state changes)
    }

    pub fn update_blink(&mut self) {
        if self.error_tab_blink && self.blink_timer > 0 {
            self.blink_timer -= 1;
            if self.blink_timer == 0 {
                self.error_tab_blink = false;
            }
        }
    }
}

pub fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    let tabs = Tabs::new(
        app.tabs
            .titles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if i == 1 && app.error_tab_blink && (app.blink_timer % 2 == 0) {
                    Span::styled(*t, Style::default().fg(Color::Red))
                } else {
                    Span::styled(*t, Style::default().fg(Color::Green))
                }
            })
            .collect::<Vec<_>>(),
    )
    .block(Block::default().borders(Borders::ALL).title(app.title))
    .select(app.tabs.index)
    .highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(tabs, chunks[0]);

    app.update_blink();

    match app.tabs.index {
        0 => render_logs(f, app, chunks[2]),
        1 => render_errors(f, app, chunks[2]),
        2 => render_details(f, app, chunks[2]),
        3 => render_status(f, app, chunks[2]),
        _ => {}
    }

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Progress"))
        .gauge_style(
            Style::default()
                .fg(Color::Green)
                .bg(Color::Black)
                .add_modifier(Modifier::ITALIC),
        )
        .ratio(app.progress);
    f.render_widget(gauge, chunks[3]);
}

fn render_details(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let paragraph = Paragraph::new(Text::from(app.details.clone()))
        .block(Block::default().borders(Borders::ALL).title("Details"));
    f.render_widget(paragraph, area);
}

fn render_status(f: &mut ratatui::Frame, _app: &App, area: ratatui::layout::Rect) {
    let status_message = "All systems operational."; // Example status
    let paragraph = Paragraph::new(Text::from(status_message))
        .block(Block::default().borders(Borders::ALL).title("Status"));
    f.render_widget(paragraph, area);
}

fn render_logs(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let logs: Vec<ListItem> = app
        .logs
        .iter()
        .map(|(level, message)| {
            let style = match *level {
                "INFO" => Style::default().fg(Color::Blue),
                "ERROR" => Style::default().fg(Color::Red),
                "SUCCESS" => Style::default().fg(Color::Green),
                "CRITICAL" => Style::default().fg(Color::Magenta),
                "WARNING" => Style::default().fg(Color::Yellow),
                _ => Style::default().fg(Color::Gray),
            };
            ListItem::new(Span::styled(message.clone(), style))
        })
        .collect();
    let logs_widget = List::new(logs).block(Block::default().borders(Borders::ALL).title("Logs"));
    f.render_widget(logs_widget, area);
}

fn render_errors(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let errors: Vec<ListItem> = app
        .errors
        .iter()
        .map(|message| {
            ListItem::new(Span::styled(
                message.clone(),
                Style::default().fg(Color::Red),
            ))
        })
        .collect();
    let errors_widget = List::new(errors)
        .block(Block::default().borders(Borders::ALL).title("Errors"))
        .style(Style::default().fg(Color::Red));
    f.render_widget(errors_widget, area);
}

pub fn render_message<B: Backend>(
    terminal: &mut Terminal<B>,
    title: &str,
    message: &str,
    color: Color,
) -> Result<(), io::Error> {
    terminal.draw(|f| {
        let area = f.area();
        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        let paragraph = Paragraph::new(Text::from(message)).block(block);
        f.render_widget(paragraph, area);
    })?;
    Ok(())
}
