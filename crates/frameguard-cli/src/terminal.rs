use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use frameguard_core::{
    capture_frame, open_video, scan, Progress, ReportFormat, Risk, ScanOptions, ScanReport,
    ScanStatus, Video,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

const ACCENT: Color = Color::Rgb(103, 232, 249);
const MUTED: Color = Color::Rgb(150, 169, 196);

pub fn run(path: Option<PathBuf>, rules: Option<PathBuf>) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Open an interactive terminal for the TUI, or use frameguard scan.");
    }
    enable_raw_mode().context("Opening terminal controls")?;
    let _restore = RestoreTerminal;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let (send, receive) = mpsc::channel();
    let mut app = App {
        path: path
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        rules: rules
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..App::default()
    };
    loop {
        while let Ok(message) = receive.try_recv() {
            match message {
                Message::Progress(progress) => app.progress = progress,
                Message::Report(result) => {
                    app.active = None;
                    match result {
                        Ok((video, report)) => {
                            app.notice = if report.status == ScanStatus::Cancelled {
                                "Scan cancelled"
                            } else {
                                "Scan complete"
                            }
                            .into();
                            app.progress = report.progress;
                            app.video = Some(video);
                            app.report = Some(report);
                        }
                        Err(error) => app.notice = error,
                    }
                }
                Message::Preview {
                    id,
                    request,
                    result,
                } => {
                    if request == app.frame_request
                        && Some(id) == app.visible().get(app.selected).map(|hit| hit.id)
                    {
                        match result {
                            Ok(pixels) => app.preview = Some(pixels),
                            Err(error) => app.notice = error,
                        }
                    }
                }
            }
        }
        if app.exiting && app.active.is_none() {
            return Ok(());
        }
        terminal.draw(|frame| app.draw(frame))?;
        if !event::poll(Duration::from_millis(55))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if let Some(active) = &app.active {
                active.store(true, Ordering::Relaxed);
            } else {
                return Ok(());
            }
            continue;
        }
        if let Some((field, value)) = &mut app.edit {
            match key.code {
                KeyCode::Esc => app.edit = None,
                KeyCode::Enter => {
                    let field = *field;
                    let value = std::mem::take(value);
                    *app.field_mut(field) = value;
                    app.edit = None;
                }
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Char(character) => value.push(character),
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Char('q') => {
                app.exiting = true;
                if let Some(active) = &app.active {
                    active.store(true, Ordering::Relaxed);
                }
            }
            KeyCode::Esc => {
                if let Some(active) = &app.active {
                    active.store(true, Ordering::Relaxed);
                    app.notice = "Cancelling scan".into();
                }
            }
            KeyCode::F(2) => app.begin_edit(Field::Path),
            KeyCode::F(3) => app.begin_edit(Field::Fps),
            KeyCode::F(4) => app.begin_edit(Field::Phrases),
            KeyCode::F(5) if app.active.is_none() => {
                if let Err(error) = app.start(&send) {
                    app.notice = format!("{error:#}");
                }
            }
            KeyCode::F(6) => app.begin_edit(Field::Range),
            KeyCode::F(7) => app.begin_edit(Field::Rules),
            KeyCode::F(8) => {
                app.filter = match app.filter {
                    None => Some(Risk::High),
                    Some(Risk::High) => Some(Risk::Medium),
                    Some(Risk::Medium) => Some(Risk::Low),
                    Some(Risk::Low) => None,
                };
                app.selected = 0;
                app.preview = None;
            }
            KeyCode::Up => {
                app.selected = app.selected.saturating_sub(1);
                app.preview = None;
            }
            KeyCode::Down => {
                app.selected = (app.selected + 1).min(app.visible().len().saturating_sub(1));
                app.preview = None;
            }
            KeyCode::Enter => app.capture(&send),
            KeyCode::Char('e') => {
                if let Err(error) = app.export() {
                    app.notice = format!("{error:#}");
                }
            }
            _ => {}
        }
    }
}

struct RestoreTerminal;

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

enum Message {
    Progress(Progress),
    Report(std::result::Result<(Video, ScanReport), String>),
    Preview {
        id: u64,
        request: u64,
        result: std::result::Result<image::GrayImage, String>,
    },
}

#[derive(Clone, Copy)]
enum Field {
    Path,
    Fps,
    Phrases,
    Range,
    Rules,
}

impl Field {
    fn title(self) -> &'static str {
        match self {
            Self::Path => "MP4 path",
            Self::Fps => "Sampling FPS",
            Self::Phrases => "Custom text (comma-separated)",
            Self::Range => "Start,end seconds (blank for whole video)",
            Self::Rules => "Wordlist JSON path (blank for defaults)",
        }
    }
}

struct App {
    path: String,
    fps: String,
    phrases: String,
    range: String,
    rules: String,
    edit: Option<(Field, String)>,
    active: Option<Arc<AtomicBool>>,
    video: Option<Video>,
    report: Option<ScanReport>,
    progress: Progress,
    notice: String,
    selected: usize,
    filter: Option<Risk>,
    preview: Option<image::GrayImage>,
    frame_request: u64,
    exiting: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            path: String::new(),
            fps: "1".into(),
            phrases: String::new(),
            range: String::new(),
            rules: String::new(),
            edit: None,
            active: None,
            video: None,
            report: None,
            progress: Progress::default(),
            notice: "Choose an MP4 and start a scan.".into(),
            selected: 0,
            filter: None,
            preview: None,
            frame_request: 0,
            exiting: false,
        }
    }
}

impl App {
    fn field_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Path => &mut self.path,
            Field::Fps => &mut self.fps,
            Field::Phrases => &mut self.phrases,
            Field::Range => &mut self.range,
            Field::Rules => &mut self.rules,
        }
    }

    fn begin_edit(&mut self, field: Field) {
        if self.active.is_none() {
            let value = self.field_mut(field).clone();
            self.edit = Some((field, value));
        }
    }

    fn start(&mut self, send: &Sender<Message>) -> Result<()> {
        if self.path.trim().is_empty() {
            bail!("Choose an MP4 path with F2.");
        }
        let path = PathBuf::from(self.path.trim().trim_matches('"'));
        let fps = self
            .fps
            .trim()
            .parse::<f64>()
            .context("Enter the sampling rate as a number")?;
        let (start, end) = if self.range.trim().is_empty() {
            (0.0, None)
        } else {
            let (start, end) = self
                .range
                .split_once(',')
                .context("Enter a time range as start,end")?;
            (
                start.trim().parse::<f64>()?,
                Some(end.trim().parse::<f64>()?),
            )
        };
        let rules = (!self.rules.trim().is_empty()).then(|| PathBuf::from(self.rules.trim()));
        let options = ScanOptions {
            fps,
            start,
            end,
            phrases: self
                .phrases
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
            rules: super::load_rules(rules.as_deref())?,
            ..ScanOptions::default()
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        self.active = Some(Arc::clone(&cancelled));
        self.notice = "Reading recording".into();
        self.progress = Progress::default();
        self.report = None;
        self.preview = None;
        self.frame_request += 1;
        self.selected = 0;
        let sender = send.clone();
        thread::spawn(move || {
            let result = (|| -> Result<_> {
                let video = open_video(path)?;
                let report = scan(&video, &options, &cancelled, |progress| {
                    let _ = sender.send(Message::Progress(progress));
                })?;
                Ok((video, report))
            })();
            let _ = sender.send(Message::Report(
                result.map_err(|error| format!("{error:#}")),
            ));
        });
        Ok(())
    }

    fn visible(&self) -> Vec<&frameguard_core::Finding> {
        self.report
            .as_ref()
            .into_iter()
            .flat_map(|report| &report.findings)
            .filter(|finding| self.filter.is_none_or(|risk| risk == finding.risk))
            .collect()
    }

    fn capture(&mut self, send: &Sender<Message>) {
        let Some(video) = self.video.clone() else {
            return;
        };
        let visible = self.visible();
        let Some(hit) = visible.get(self.selected) else {
            return;
        };
        let id = hit.id;
        let time = hit.first;
        let redaction = hit.secret.then_some(hit.region);
        self.frame_request += 1;
        let request = self.frame_request;
        let sender = send.clone();
        self.notice = format!("Opening frame at {time:.2}s");
        thread::spawn(move || {
            let result = (|| -> Result<_> {
                let mut pixels = image::load_from_memory(&capture_frame(&video, time)?)?.to_luma8();
                if let Some(region) = redaction {
                    let left = (region.x * f64::from(pixels.width())).floor() as u32;
                    let right = ((region.x + region.w) * f64::from(pixels.width())).ceil() as u32;
                    let top = (region.y * f64::from(pixels.height())).floor() as u32;
                    let bottom = ((region.y + region.h) * f64::from(pixels.height())).ceil() as u32;
                    for y in top..bottom.min(pixels.height()) {
                        for x in left..right.min(pixels.width()) {
                            pixels.put_pixel(x, y, image::Luma([0]));
                        }
                    }
                }
                Ok(image::imageops::thumbnail(&pixels, 84, 42))
            })();
            let _ = sender.send(Message::Preview {
                id,
                request,
                result: result.map_err(|error| format!("{error:#}")),
            });
        });
    }

    fn export(&mut self) -> Result<()> {
        let mut report = self.report.clone().context("Run a scan before exporting")?;
        report
            .findings
            .retain(|finding| self.filter.is_none_or(|risk| finding.risk == risk));
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let path = PathBuf::from(format!("frameguard-findings-{seconds}.json"));
        super::write_report_file(&path, &report, ReportFormat::Json, false)?;
        self.notice = format!("Saved {}", path.display());
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        let regions = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(7),
            Constraint::Length(5),
        ])
        .split(frame.area());
        let title = Line::from(vec![
            Span::styled(
                " FrameGuard ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if self.active.is_some() {
                    " SCANNING"
                } else {
                    " LOCAL VIDEO SCAN"
                },
                Style::default().fg(MUTED),
            ),
        ]);
        let filename = if self.path.is_empty() {
            "F2  Choose an MP4"
        } else {
            &self.path
        };
        frame.render_widget(
            Paragraph::new(vec![title, Line::from(filename)]).block(Block::bordered()),
            regions[0],
        );
        let constraints = if regions[1].width >= 92 {
            Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
        } else {
            Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)])
        };
        let panes = constraints.split(regions[1]);
        let visible = self.visible();
        let rows = visible
            .iter()
            .map(|hit| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:7.2}s   ", hit.first), Style::default().fg(MUTED)),
                    Span::styled(
                        format!("{:6} ", hit.risk.as_str()),
                        Style::default().fg(risk_color(hit.risk)),
                    ),
                    Span::raw(&hit.name),
                ]))
            })
            .collect::<Vec<_>>();
        let filter = self.filter.map(Risk::as_str).unwrap_or("all");
        let heading = format!(" Findings  {}  /  {filter} ", visible.len());
        let mut selection =
            ListState::default().with_selected((!visible.is_empty()).then_some(self.selected));
        frame.render_stateful_widget(
            List::new(rows)
                .block(Block::bordered().title(heading))
                .highlight_style(Style::default().bg(Color::Rgb(26, 57, 71)).fg(Color::White)),
            panes[0],
            &mut selection,
        );

        let mut info = Vec::new();
        if let Some(hit) = visible.get(self.selected) {
            info.push(Line::from(hit.name.as_str()).bold());
            info.push(Line::from(format!(
                "{:.2}s to {:.2}s   {} hits",
                hit.first, hit.last, hit.samples_hit
            )));
            info.push(Line::from(if hit.secret {
                "[redacted]"
            } else {
                &hit.text
            }));
            if let Some(preview) = &self.preview {
                info.extend(pixel_rows(
                    preview,
                    panes[1].width.saturating_sub(4),
                    panes[1].height.saturating_sub(5),
                ));
            } else {
                info.push(Line::from("Enter opens the sampled frame").fg(MUTED));
            }
        } else {
            info.push(Line::from(format!(
                "{} FPS   {}",
                self.fps,
                if self.range.is_empty() {
                    "whole video"
                } else {
                    &self.range
                }
            )));
            info.push(Line::from(format!(
                "Custom text   {}",
                if self.phrases.is_empty() {
                    "none"
                } else {
                    &self.phrases
                }
            )));
            info.push(Line::from(format!(
                "Wordlist   {}",
                if self.rules.is_empty() {
                    "default"
                } else {
                    &self.rules
                }
            )));
        }
        frame.render_widget(
            Paragraph::new(info)
                .block(Block::bordered().title(" Inspect "))
                .wrap(Wrap { trim: false }),
            panes[1],
        );

        let bottom = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(regions[2]);
        let ratio = if self.progress.total > 0 {
            self.progress.done as f64 / self.progress.total as f64
        } else {
            0.0
        };
        frame.render_widget(
            Gauge::default()
                .ratio(ratio.clamp(0.0, 1.0))
                .gauge_style(Style::default().fg(ACCENT))
                .label(format!(
                    "{} / {} samples    {} reused    {} findings",
                    self.progress.done,
                    self.progress.total,
                    self.progress.reused,
                    self.progress.findings
                )),
            bottom[0],
        );
        frame.render_widget(
            Paragraph::new(self.notice.as_str()).style(Style::default().fg(MUTED)),
            bottom[1],
        );
        if let Some((field, value)) = &self.edit {
            frame.render_widget(
                Paragraph::new(value.as_str()).block(Block::bordered().title(field.title())),
                bottom[2],
            );
        } else {
            let shortcuts = "F2 Video   F3 FPS   F4 Text   F5 Scan   F6 Range   F7 Rules   F8 Risk   Enter Frame   E Export   Esc Cancel   Q Exit";
            frame.render_widget(
                Paragraph::new(shortcuts)
                    .block(Block::bordered())
                    .wrap(Wrap { trim: true }),
                bottom[2],
            );
        }
    }
}

fn risk_color(risk: Risk) -> Color {
    match risk {
        Risk::High => Color::LightRed,
        Risk::Medium => Color::LightYellow,
        Risk::Low => Color::LightGreen,
    }
}

fn pixel_rows(preview: &image::GrayImage, width: u16, height: u16) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let image = image::imageops::thumbnail(preview, u32::from(width), u32::from(height) * 2);
    let shade = |value| Color::Rgb(value, value, value);
    (0..image.height())
        .step_by(2)
        .map(|y| {
            let spans = (0..image.width())
                .map(|x| {
                    let upper = image.get_pixel(x, y)[0];
                    let lower = image.get_pixel(x, (y + 1).min(image.height() - 1))[0];
                    Span::styled("▀", Style::default().fg(shade(upper)).bg(shade(lower)))
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect()
}
