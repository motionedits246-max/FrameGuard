use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    media::{capture_output, diagnostic, wait_for_process, ScanTools},
    rules::{matchers, Matcher},
    Risk, Rule, Video, VideoMetadata,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ScanOptions {
    pub fps: f64,
    pub start: f64,
    pub end: Option<f64>,
    pub phrases: Vec<String>,
    pub rules: Vec<Rule>,
    pub language: String,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            fps: 1.0,
            start: 0.0,
            end: None,
            phrases: Vec::new(),
            rules: crate::default_rules(),
            language: "eng".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub id: u64,
    pub name: String,
    pub risk: Risk,
    pub text: String,
    pub secret: bool,
    pub custom: bool,
    pub masked: bool,
    pub first: f64,
    pub last: f64,
    pub samples_hit: u64,
    pub sample_times_ms: Vec<u64>,
    pub region: Region,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanStatus {
    Complete,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub reused: u64,
    pub time: f64,
    pub findings: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanReport {
    pub video: VideoMetadata,
    pub start: f64,
    pub end: f64,
    pub fps: f64,
    pub status: ScanStatus,
    pub progress: Progress,
    pub findings: Vec<Finding>,
}

pub fn scan(
    video: &Video,
    options: &ScanOptions,
    cancelled: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<ScanReport> {
    let end = options.end.unwrap_or(video.metadata.duration);
    let fps = options.fps;
    if !fps.is_finite() || !(0.1..=30.0).contains(&fps) {
        bail!("Choose a sampling rate from 0.1 to 30 FPS.");
    }
    if !options.start.is_finite()
        || !end.is_finite()
        || options.start < 0.0
        || end <= options.start
        || end > video.metadata.duration
    {
        bail!("Choose a time range inside the video.");
    }
    if options.language.is_empty()
        || !options
            .language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
    {
        bail!("Enter an installed Tesseract language code, such as eng.");
    }
    let matchers = matchers(&options.rules, &options.phrases)?;
    let total = (((end - options.start) * fps - 1e-9).ceil() as u64).max(1);
    let mut report = ScanReport {
        video: video.metadata.clone(),
        start: options.start,
        end,
        fps,
        status: ScanStatus::Complete,
        progress: Progress {
            total,
            time: options.start,
            ..Progress::default()
        },
        findings: Vec::new(),
    };
    on_progress(report.progress);
    if cancelled.load(Ordering::Relaxed) {
        report.status = ScanStatus::Cancelled;
        return Ok(report);
    }

    let mut decoder = video
        .tools
        .ffmpeg()
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-ss",
        ])
        .arg(format!("{:.9}", options.start))
        .arg("-i")
        .arg(video.path())
        .args(["-map", "0:V:0", "-an", "-sn", "-dn", "-vf"])
        .arg(format!(
            "fps=fps={fps}:start_time=0:round=up:eof_action=pass,format=gray"
        ))
        .args([
            "-frames:v",
            &total.to_string(),
            "-threads",
            "1",
            "-c:v",
            "pgm",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Starting FFmpeg")?;
    let pipe = decoder.stdout.take().expect("piped FFmpeg output");
    let stderr = decoder.stderr.take().expect("piped FFmpeg errors");
    let (tx, rx) = mpsc::sync_channel(1);
    let frames = thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        loop {
            let frame = match read_frame(&mut reader) {
                Ok(Some(frame)) => Ok(frame),
                Ok(None) => break,
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            };
            if tx.send(frame).is_err() {
                break;
            }
        }
    });
    let errors = thread::spawn(move || capture_output(stderr, 32 * 1024));
    let result = scan_frames(
        &rx,
        &matchers,
        &video.tools,
        options,
        cancelled,
        &mut report,
        &mut on_progress,
    );
    if result.is_err() || report.status == ScanStatus::Cancelled {
        let _ = decoder.kill();
    }
    drop(rx);
    let exit = decoder.wait();
    frames
        .join()
        .map_err(|_| anyhow!("The decoder reader stopped unexpectedly."))?;
    let notes = errors
        .join()
        .map_err(|_| anyhow!("The decoder error reader stopped unexpectedly."))??;
    result?;
    if report.status == ScanStatus::Complete {
        if !exit?.success() {
            bail!("FFmpeg could not decode the video: {}", diagnostic(&notes));
        }
        if report.progress.done == 0 {
            bail!("FFmpeg found no sampled frames in the selected range.");
        }
    }
    Ok(report)
}

type MatchKey = (usize, String, i32, i32);

fn scan_frames(
    frames: &Receiver<Result<Frame>>,
    matchers: &[Matcher],
    tools: &ScanTools,
    options: &ScanOptions,
    cancelled: &AtomicBool,
    report: &mut ScanReport,
    on_progress: &mut impl FnMut(Progress),
) -> Result<()> {
    let mut cache = HashMap::new();
    let mut previous = HashMap::new();
    while report.progress.done < report.progress.total {
        if cancelled.load(Ordering::Relaxed) {
            report.status = ScanStatus::Cancelled;
            break;
        }
        let frame = match frames.recv_timeout(Duration::from_millis(70)) {
            Ok(frame) => frame?,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let time = report.start + report.progress.done as f64 / report.fps;
        let hash = blake3::hash(&frame.pgm);
        if cache.contains_key(&hash) {
            report.progress.reused += 1;
        } else {
            let Some(lines) = read_ocr(tools, &frame, &options.language, cancelled)
                .with_context(|| format!("Reading text at {time:.2} seconds"))?
            else {
                report.status = ScanStatus::Cancelled;
                break;
            };
            if cache.len() == 256 {
                cache.clear();
            }
            cache.insert(hash, lines);
        }
        previous = add_findings(
            &cache[&hash],
            matchers,
            previous,
            &mut report.findings,
            time,
        );
        report.progress.done += 1;
        report.progress.time = time;
        report.progress.findings = report.findings.len();
        on_progress(report.progress);
    }
    Ok(())
}

fn add_findings(
    lines: &[OcrLine],
    matchers: &[Matcher],
    mut previous: HashMap<MatchKey, usize>,
    findings: &mut Vec<Finding>,
    time: f64,
) -> HashMap<MatchKey, usize> {
    let mut current = HashMap::new();
    for line in lines {
        for (index, rule) in matchers.iter().enumerate() {
            for hit in rule
                .regex
                .find_iter(&line.text)
                .filter(|hit| !hit.is_empty())
            {
                let Some(region) = line.region(hit.start(), hit.end()) else {
                    continue;
                };
                let key = (
                    index,
                    hit.as_str().to_owned(),
                    (region.x * 100.0).round() as i32,
                    (region.y * 100.0).round() as i32,
                );
                if current.contains_key(&key) {
                    continue;
                }
                let finding = if let Some(index) = previous.remove(&key) {
                    findings[index].last = time;
                    findings[index].samples_hit += 1;
                    findings[index]
                        .sample_times_ms
                        .push((time * 1000.0).round() as u64);
                    index
                } else {
                    let index = findings.len();
                    findings.push(Finding {
                        id: index as u64 + 1,
                        name: rule.name.clone(),
                        risk: rule.risk,
                        text: hit.as_str().into(),
                        secret: rule.secret,
                        custom: rule.custom,
                        masked: false,
                        first: time,
                        last: time,
                        samples_hit: 1,
                        sample_times_ms: vec![(time * 1000.0).round() as u64],
                        region,
                    });
                    index
                };
                current.insert(key, finding);
            }
        }
    }
    current
}

struct Frame {
    width: u32,
    height: u32,
    pgm: Vec<u8>,
}

fn read_frame(reader: &mut impl BufRead) -> Result<Option<Frame>> {
    let Some(magic) = read_token(reader)? else {
        return Ok(None);
    };
    if magic != "P5" {
        bail!("FFmpeg sent an unexpected image format.");
    }
    let width: u32 = read_token(reader)?
        .context("Missing frame width")?
        .parse()?;
    let height: u32 = read_token(reader)?
        .context("Missing frame height")?
        .parse()?;
    let depth = read_token(reader)?.context("Missing frame bit depth")?;
    let pixels = u64::from(width) * u64::from(height);
    if depth != "255" || pixels == 0 || pixels > 64_000_000 {
        bail!("The decoded frame resolution or bit depth is unsupported.");
    }
    let mut pgm = format!("P5\n{width} {height}\n255\n").into_bytes();
    let header = pgm.len();
    pgm.resize(header + pixels as usize, 0);
    reader
        .read_exact(&mut pgm[header..])
        .context("Reading a video frame")?;
    Ok(Some(Frame { width, height, pgm }))
}

fn read_token(reader: &mut impl BufRead) -> Result<Option<String>> {
    let mut token = Vec::new();
    let mut byte = [0];
    while reader.read(&mut byte)? != 0 {
        match byte[0] {
            b'#' if token.is_empty() => {
                let mut comment = Vec::new();
                reader.read_until(b'\n', &mut comment)?;
            }
            value if value.is_ascii_whitespace() => {
                if !token.is_empty() {
                    break;
                }
            }
            value => {
                if token.len() == 32 {
                    bail!("A decoded frame has an invalid header.");
                }
                token.push(value);
            }
        }
    }
    if token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(String::from_utf8(token)?))
    }
}

fn read_ocr(
    tools: &ScanTools,
    frame: &Frame,
    language: &str,
    cancelled: &AtomicBool,
) -> Result<Option<Vec<OcrLine>>> {
    let mut child = tools
        .tesseract()
        .args([
            "stdin", "stdout", "-l", language, "--psm", "6", "--dpi", "300", "tsv",
        ])
        .env("OMP_THREAD_LIMIT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Starting Tesseract")?;
    let mut input = child.stdin.take().expect("piped OCR input");
    let output = child.stdout.take().expect("piped OCR output");
    let errors = child.stderr.take().expect("piped OCR errors");
    thread::scope(|scope| {
        let send = scope.spawn(move || input.write_all(&frame.pgm));
        let receive = scope.spawn(move || capture_output(output, 8 * 1024 * 1024));
        let notes = scope.spawn(move || capture_output(errors, 32 * 1024));
        let exit = wait_for_process(&mut child, cancelled)?;
        let sent = send
            .join()
            .map_err(|_| anyhow!("The OCR input reader stopped unexpectedly."))?;
        let output = receive
            .join()
            .map_err(|_| anyhow!("The OCR output reader stopped unexpectedly."))?;
        let notes = notes
            .join()
            .map_err(|_| anyhow!("The OCR error reader stopped unexpectedly."))?;
        let Some(exit) = exit else { return Ok(None) };
        if !exit.success() {
            bail!("Tesseract failed: {}", diagnostic(&notes?));
        }
        sent?;
        let tsv = String::from_utf8(output?)?;
        Ok(Some(parse_tsv(&tsv, frame.width, frame.height)))
    })
}

#[derive(Default)]
struct OcrLine {
    text: String,
    words: Vec<OcrWord>,
}

struct OcrWord {
    start: usize,
    end: usize,
    region: Region,
}

impl OcrLine {
    fn region(&self, start: usize, end: usize) -> Option<Region> {
        self.words
            .iter()
            .filter(|word| word.start < end && word.end > start)
            .map(|word| word.region)
            .reduce(|a, b| {
                let x = a.x.min(b.x);
                let y = a.y.min(b.y);
                Region {
                    x,
                    y,
                    w: (a.x + a.w).max(b.x + b.w) - x,
                    h: (a.y + a.h).max(b.y + b.h) - y,
                }
            })
    }
}

fn parse_tsv(tsv: &str, width: u32, height: u32) -> Vec<OcrLine> {
    let mut lines: Vec<OcrLine> = Vec::new();
    let mut groups = HashMap::new();
    for row in tsv.lines() {
        let columns: Vec<_> = row.splitn(12, '\t').collect();
        if columns.len() != 12 || columns[0] != "5" || columns[11].trim().is_empty() {
            continue;
        }
        let number = |index: usize| columns[index].parse::<u32>().unwrap_or_default();
        let x = number(6).min(width);
        let y = number(7).min(height);
        let w = number(8).min(width - x);
        let h = number(9).min(height - y);
        if w == 0 || h == 0 {
            continue;
        }
        let key = (columns[1], columns[2], columns[3], columns[4]);
        let position = *groups.entry(key).or_insert_with(|| {
            lines.push(OcrLine::default());
            lines.len() - 1
        });
        let line = &mut lines[position];
        if !line.text.is_empty() {
            line.text.push(' ');
        }
        let start = line.text.len();
        line.text.push_str(columns[11].trim());
        line.words.push(OcrWord {
            start,
            end: line.text.len(),
            region: Region {
                x: f64::from(x) / f64::from(width),
                y: f64::from(y) / f64::from(height),
                w: f64::from(w) / f64::from(width),
                h: f64::from(h) / f64::from(height),
            },
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_ends_when_the_text_disappears() {
        let tsv = "5\t1\t1\t1\t1\t1\t100\t200\t400\t20\t99\tAPI KEY: DEMO_VISIBLE_SAMPLE_1234";
        let lines = parse_tsv(tsv, 1000, 500);
        let rules = matchers(&crate::default_rules(), &[]).unwrap();
        let mut hits = Vec::new();
        let cursor = add_findings(&lines, &rules, HashMap::new(), &mut hits, 1.0);
        let cursor = add_findings(&lines, &rules, cursor, &mut hits, 2.0);
        let cursor = add_findings(&[], &rules, cursor, &mut hits, 3.0);
        add_findings(&lines, &rules, cursor, &mut hits, 4.0);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].samples_hit, 2);
        assert_eq!(hits[0].last, 2.0);
        assert_eq!(hits[0].region.x, 0.1);
        assert_eq!(hits[1].first, 4.0);
    }
}
