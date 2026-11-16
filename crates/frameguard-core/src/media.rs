use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct Video {
    path: PathBuf,
    pub metadata: VideoMetadata,
}

impl Video {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoMetadata {
    pub name: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
}

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Stream {
    width: u32,
    height: u32,
    avg_frame_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}

pub fn open_video(path: impl AsRef<Path>) -> Result<Video> {
    let path = dunce::canonicalize(path).context("Opening the selected video")?;
    if !path.is_file()
        || !path
            .extension()
            .and_then(|part| part.to_str())
            .is_some_and(|part| part.eq_ignore_ascii_case("mp4"))
    {
        bail!("Choose a local MP4 file.");
    }
    let output = ffprobe()
        .args([
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-select_streams",
            "V:0",
            "-show_entries",
            "stream=width,height,avg_frame_rate,duration:format=duration",
            "-of",
            "json",
        ])
        .arg(&path)
        .output()
        .context("Running ffprobe. Install FFmpeg or set FRAMEGUARD_FFPROBE")?;
    if !output.status.success() {
        bail!(
            "ffprobe could not read the MP4: {}",
            diagnostic(&output.stderr)
        );
    }
    let probe: Probe = serde_json::from_slice(&output.stdout).context("Reading video metadata")?;
    let stream = probe
        .streams
        .first()
        .context("The MP4 has no video stream.")?;
    let valid_duration = |value: &str| {
        value
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n > 0.0)
    };
    let duration = stream
        .duration
        .as_deref()
        .and_then(valid_duration)
        .or_else(|| {
            probe
                .format
                .as_ref()?
                .duration
                .as_deref()
                .and_then(valid_duration)
        })
        .context("The video stream has no readable duration.")?;
    if stream.width == 0 || stream.height == 0 {
        bail!("The video stream has an invalid resolution.");
    }
    let fps = stream.avg_frame_rate.as_deref().and_then(|rate| {
        let (numerator, denominator) = rate.split_once('/')?;
        let value = numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?;
        (value.is_finite() && value > 0.0).then_some(value)
    });
    let name = path
        .file_name()
        .context("Reading the video filename")?
        .to_string_lossy()
        .into_owned();
    Ok(Video {
        path,
        metadata: VideoMetadata {
            name,
            duration,
            width: stream.width,
            height: stream.height,
            fps,
        },
    })
}

pub fn capture_frame(video: &Video, seconds: f64) -> Result<Vec<u8>> {
    if !seconds.is_finite() || seconds < 0.0 || seconds >= video.metadata.duration {
        bail!("The timestamp is outside the video.");
    }
    let output = ffmpeg()
        .args(["-v", "error", "-protocol_whitelist", "file,pipe", "-ss"])
        .arg(format!("{seconds:.9}"))
        .arg("-i")
        .arg(video.path())
        .args([
            "-map",
            "0:V:0",
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .output()
        .context("Running FFmpeg for the selected frame")?;
    if !output.status.success() || output.stdout.is_empty() {
        bail!(
            "FFmpeg could not extract this frame: {}",
            diagnostic(&output.stderr)
        );
    }
    Ok(output.stdout)
}

pub(crate) fn ffmpeg() -> Command {
    command("FRAMEGUARD_FFMPEG", "ffmpeg")
}

fn ffprobe() -> Command {
    command("FRAMEGUARD_FFPROBE", "ffprobe")
}

pub(crate) fn tesseract() -> Command {
    command("FRAMEGUARD_TESSERACT", "tesseract")
}

fn command(variable: &str, fallback: &str) -> Command {
    let executable = env::var_os(variable).unwrap_or_else(|| fallback.into());
    let mut command = Command::new(executable);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

pub(crate) fn diagnostic(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.trim().chars().take(500).collect()
}
