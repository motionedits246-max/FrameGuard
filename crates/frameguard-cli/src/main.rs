mod terminal;

use std::{
    fs::File,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use frameguard_core::{
    default_rules, open_video, scan, write_report, ReportFormat, Rule, ScanOptions, ScanStatus,
};

#[derive(Parser)]
#[command(version, about = "Find sensitive text in local screen recordings")]
struct Cli {
    #[command(subcommand)]
    command: Option<Mode>,
}

#[derive(Subcommand)]
enum Mode {
    /// Scan a local MP4.
    Scan(ScanArgs),
    /// Open the interactive terminal.
    Tui {
        video: Option<PathBuf>,
        #[arg(long)]
        rules: Option<PathBuf>,
    },
    /// Print the default wordlist as JSON.
    Rules,
}

#[derive(Args)]
struct ScanArgs {
    video: PathBuf,
    #[arg(long, default_value_t = 1.0)]
    fps: f64,
    /// Starting timestamp in seconds.
    #[arg(long, default_value_t = 0.0)]
    start: f64,
    /// Ending timestamp in seconds.
    #[arg(long)]
    end: Option<f64>,
    /// Case-insensitive text to find. Repeat for multiple phrases.
    #[arg(long = "text")]
    phrases: Vec<String>,
    #[arg(long)]
    rules: Option<PathBuf>,
    #[arg(long, default_value = "eng")]
    language: String,
    #[arg(long, value_enum, default_value = "json")]
    format: OutputFormat,
    #[arg(long)]
    output: Option<PathBuf>,
    /// Include matched secret values in the report.
    #[arg(long)]
    show_secrets: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Json,
    Csv,
}

impl From<OutputFormat> for ReportFormat {
    fn from(value: OutputFormat) -> Self {
        match value {
            OutputFormat::Json => Self::Json,
            OutputFormat::Csv => Self::Csv,
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("FrameGuard: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        None => terminal::run(None, None)?,
        Some(Mode::Tui { video, rules }) => terminal::run(video, rules)?,
        Some(Mode::Rules) => {
            let mut out = io::stdout().lock();
            serde_json::to_writer_pretty(&mut out, &default_rules())?;
            writeln!(out)?;
        }
        Some(Mode::Scan(args)) => return run_scan(args),
    }
    Ok(ExitCode::SUCCESS)
}

fn run_scan(args: ScanArgs) -> Result<ExitCode> {
    let video = open_video(args.video)?;
    let options = ScanOptions {
        fps: args.fps,
        start: args.start,
        end: args.end,
        phrases: args.phrases,
        rules: load_rules(args.rules.as_deref())?,
        language: args.language,
    };
    if let Some(path) = &args.output {
        if path.canonicalize().ok().as_deref() == video.path().canonicalize().ok().as_deref() {
            bail!("Choose a report path separate from the video.");
        }
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&cancelled);
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))
        .context("Setting up Ctrl+C cancellation")?;
    let progress_enabled = io::stderr().is_terminal();
    let mut last_status = Instant::now();
    let report = scan(&video, &options, &cancelled, |progress| {
        if progress_enabled
            && (last_status.elapsed() >= Duration::from_millis(250)
                || progress.done == progress.total)
        {
            eprint!(
                "\r{} / {} samples  {} reused  {} findings    ",
                progress.done, progress.total, progress.reused, progress.findings
            );
            last_status = Instant::now();
        }
    })?;
    if progress_enabled {
        eprintln!();
    }
    if let Some(path) = &args.output {
        write_report_file(path, &report, args.format.into(), args.show_secrets)?;
    } else {
        let mut out = io::stdout().lock();
        write_report(&mut out, &report, args.format.into(), args.show_secrets)?;
        if matches!(args.format, OutputFormat::Json) {
            writeln!(&mut out)?;
        }
    }
    Ok(if report.status == ScanStatus::Cancelled {
        ExitCode::from(130)
    } else {
        ExitCode::SUCCESS
    })
}

pub(crate) fn load_rules(path: Option<&Path>) -> Result<Vec<Rule>> {
    match path {
        Some(path) => {
            let file =
                File::open(path).with_context(|| format!("Opening wordlist {}", path.display()))?;
            serde_json::from_reader(file).context("Reading the JSON wordlist")
        }
        None => Ok(default_rules()),
    }
}

pub(crate) fn write_report_file(
    path: &Path,
    report: &frameguard_core::ScanReport,
    format: ReportFormat,
    reveal: bool,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|name| !name.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file =
        tempfile::NamedTempFile::new_in(parent).context("Opening the report destination")?;
    write_report(file.as_file_mut(), report, format, reveal)?;
    file.persist(path)
        .with_context(|| format!("Saving report {}", path.display()))?;
    Ok(())
}
