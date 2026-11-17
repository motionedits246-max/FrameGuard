use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use frameguard_core::{ScanTools, WINDOWS_RUNTIME_DIR};
use reqwest::blocking::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

struct Asset {
    label: &'static str,
    url: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const FFMPEG: Asset = Asset {
    label: "Downloading FFmpeg and ffprobe",
    url: "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.7z",
    bytes: 35_430_500,
    sha256: "4705843ccaaf54257c16ad90f3e952ece33c17df964ecf7bfdbb0f49c7171077",
};

const TESSERACT: Asset = Asset {
    label: "Downloading Tesseract OCR",
    url: "https://github.com/tesseract-ocr/tesseract/releases/download/5.5.3/tesseract-ocr-w64-setup-5.5.3.20260724.exe",
    bytes: 26_573_224,
    sha256: "bee9e3434bd94fd65387d9be28cd467a41f61b1275383b55b0f59a1331270ae4",
};

const ENGLISH: Asset = Asset {
    label: "Downloading English recognition data",
    url: "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/4.1.0/eng.traineddata",
    bytes: 4_113_088,
    sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
};

const DOWNLOAD_BYTES: u64 = FFMPEG.bytes + TESSERACT.bytes + ENGLISH.bytes;
const FFMPEG_ROOT: &str = "ffmpeg-9.0.2-essentials_build";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Serialize)]
struct SetupProgress {
    stage: &'static str,
    percent: u8,
    downloaded: u64,
    total: u64,
}

pub(super) fn install_for_app(app: &AppHandle) -> Result<()> {
    let data = app.path().app_local_data_dir()?;
    let archive_tool = app
        .path()
        .resource_dir()?
        .join("archive-tool")
        .join("7z.exe");
    install(&data, &archive_tool, |progress| {
        let _ = app.emit_to("main", "setup-progress", progress);
    })
}

fn install(
    data: &Path,
    archive_tool: &Path,
    mut on_progress: impl FnMut(SetupProgress),
) -> Result<()> {
    fs::create_dir_all(data).context("Creating the FrameGuard application folder")?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join("scanner-setup.lock"))
        .context("Opening the scanner setup lock")?;
    lock.try_lock()
        .context("Another FrameGuard window is setting up the scanner. Close it and retry.")?;

    let mut progress = |stage, percent, downloaded| {
        on_progress(SetupProgress {
            stage,
            percent,
            downloaded,
            total: DOWNLOAD_BYTES,
        });
    };
    let installed = data.join(WINDOWS_RUNTIME_DIR);
    let tools = ScanTools::managed(&installed);
    if tools.has_managed_files() && tools.verify().is_ok() {
        progress("Scanner components ready", 100, DOWNLOAD_BYTES);
        return Ok(());
    }
    if !archive_tool.is_file() || !archive_tool.with_file_name("7z.dll").is_file() {
        bail!("FrameGuard's archive tool is missing. Run the FrameGuard Windows installer again.");
    }
    for entry in fs::read_dir(data)?.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".scanner-setup-")
            && entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    let staging = tempfile::Builder::new()
        .prefix(".scanner-setup-")
        .tempdir_in(data)
        .context("Creating a temporary scanner setup folder")?;
    let ready = staging.path().join("ready");
    let ffmpeg_dir = ready.join("ffmpeg");
    let tesseract_dir = ready.join("tesseract");
    let ffmpeg_file = staging.path().join("ffmpeg.7z");
    let ocr_file = staging.path().join("tesseract-setup.exe");
    let language_file = staging.path().join("eng.traineddata");
    let client = Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(1800))
        .user_agent(concat!("FrameGuard/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Starting the scanner download client")?;

    let mut downloaded = 0;
    for (asset, file) in [
        (&FFMPEG, &ffmpeg_file),
        (&TESSERACT, &ocr_file),
        (&ENGLISH, &language_file),
    ] {
        download(&client, asset, file, |received| {
            let total = downloaded + received;
            progress(asset.label, (total * 70 / DOWNLOAD_BYTES) as u8, total);
        })
        .with_context(|| asset.label.to_owned())?;
        downloaded += asset.bytes;
    }

    progress("Installing FFmpeg and ffprobe", 72, downloaded);
    extract(
        archive_tool,
        &ffmpeg_file,
        &ffmpeg_dir,
        "e",
        &[
            &format!("{FFMPEG_ROOT}/bin/ffmpeg.exe"),
            &format!("{FFMPEG_ROOT}/bin/ffprobe.exe"),
            &format!("{FFMPEG_ROOT}/LICENSE"),
        ],
    )
    .context("Unpacking FFmpeg and ffprobe")?;

    progress("Installing Tesseract OCR", 85, downloaded);
    extract(
        archive_tool,
        &ocr_file,
        &tesseract_dir,
        "x",
        &[
            "tesseract.exe",
            "*.dll",
            "tessdata/configs/tsv",
            "doc/LICENSE",
        ],
    )
    .context("Unpacking Tesseract OCR")?;
    fs::rename(
        language_file,
        tesseract_dir.join("tessdata").join("eng.traineddata"),
    )
    .context("Installing English recognition data")?;

    progress("Checking video decoding and OCR", 96, downloaded);
    ScanTools::managed(&ready)
        .verify()
        .context("Testing the scanner components")?;
    if installed.exists() {
        fs::remove_dir_all(&installed).context("Removing the damaged scanner components")?;
    }
    fs::rename(ready, installed).context("Activating the installed scanner components")?;
    progress("Scanner components ready", 100, downloaded);
    Ok(())
}

fn download(
    client: &Client,
    asset: &Asset,
    file: &Path,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    progress(0);
    let mut response = client.get(asset.url).send()?.error_for_status()?;
    let mut output = File::create(file)?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 64 * 1024];
    let mut last_update = Instant::now();
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        if size > asset.bytes {
            bail!("The downloaded file is larger than expected.");
        }
        output.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
        if last_update.elapsed() >= Duration::from_millis(100) {
            progress(size);
            last_update = Instant::now();
        }
    }
    if size != asset.bytes || format!("{:x}", hash.finalize()) != asset.sha256 {
        bail!("The download did not pass its integrity check. Retry setup.");
    }
    progress(size);
    Ok(())
}

fn extract(tool: &Path, archive: &Path, dir: &Path, mode: &str, files: &[&str]) -> Result<()> {
    fs::create_dir_all(dir)?;
    let mut output_dir = std::ffi::OsString::from("-o");
    output_dir.push(dir);
    let result = Command::new(tool)
        .arg(mode)
        .arg(archive)
        .args(files)
        .arg(output_dir)
        .args(["-y", "-bso0", "-bsp0"])
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("Starting FrameGuard's archive tool")?;
    if !result.status.success() {
        bail!(
            "Archive extraction failed ({}): {}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use frameguard_core::{open_video_with_tools, scan, ScanOptions, ScanStatus};

    use super::*;

    #[test]
    fn installs_and_scans_with_managed_tools() -> Result<()> {
        let data = tempfile::tempdir()?;
        let archive_tool =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/archive-tool/7z.exe");
        let mut progress = 0;
        install(data.path(), &archive_tool, |update| {
            assert!(update.percent >= progress);
            progress = update.percent;
        })?;
        assert_eq!(progress, 100);

        let runtime = data.path().join(WINDOWS_RUNTIME_DIR);
        let video = data.path().join("fixture.mp4");
        let output = Command::new(runtime.join("ffmpeg/ffmpeg.exe"))
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=white:s=320x200:d=1",
                "-c:v",
                "libx264",
            ])
            .arg(&video)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let video = open_video_with_tools(video, ScanTools::managed(&runtime))?;
        let report = scan(
            &video,
            &ScanOptions::default(),
            &AtomicBool::new(false),
            |_| {},
        )?;
        assert_eq!(report.status, ScanStatus::Complete);
        assert_eq!(report.progress.done, 1);

        install(data.path(), &archive_tool, |update| {
            assert_eq!(update.percent, 100)
        })?;
        Ok(())
    }
}
