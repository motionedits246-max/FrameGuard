# FrameGuard

FrameGuard scans local MP4 screen recordings for sensitive text before you publish or share them. Use it to catch credentials, email addresses, client names, private asset filenames, or phrases you define.

[Download FrameGuard](https://github.com/motionedits246-max/FrameGuard/releases/latest) · [Application source](https://github.com/motionedits246-max/FrameGuard/tree/main) · [Website source](https://github.com/motionedits246-max/FrameGuard/tree/website)

## Choose a build

| Platform | Desktop (GUI) | Terminal (TUI and CLI) |
| --- | --- | --- |
| Windows x64 | [Setup EXE](https://github.com/motionedits246-max/FrameGuard/releases/latest/download/FrameGuard_Windows_x64_Setup.exe) | [Portable ZIP](https://github.com/motionedits246-max/FrameGuard/releases/latest/download/FrameGuard_Windows_x64_Terminal.zip) |
| Debian 12 and Ubuntu 22.04 or newer, x64 | [Desktop DEB](https://github.com/motionedits246-max/FrameGuard/releases/latest/download/FrameGuard_Linux_x64_Desktop.deb) | [Terminal DEB](https://github.com/motionedits246-max/FrameGuard/releases/latest/download/FrameGuard_Linux_x64_Terminal.deb) |

### Desktop

On Windows, run the setup EXE, open FrameGuard, and choose **Download & install** when it checks for scanner components. The optional keyword setup can suggest terms for your work or take your own.

On Debian or Ubuntu, install the downloaded package using apt so it can resolve scanner and playback dependencies.

```bash
sudo apt install ./FrameGuard_Linux_x64_Desktop.deb
```

Open an MP4, set the sampling rate, and scan the full recording or a selected time range. Click a finding to review its flagged clip. Use the Wordlist to adjust rules and severity, then export findings as JSON or CSV.

### Terminal

On Windows, extract the ZIP and start the terminal from its folder.

```powershell
.\frameguard.exe tui
```

The Windows terminal build uses scanner components already installed by the desktop app or FFmpeg and Tesseract available on PATH. A standalone installation needs `ffmpeg`, `ffprobe`, and Tesseract with English OCR data. [FFmpeg downloads](https://ffmpeg.org/download.html) · [Tesseract installation](https://tesseract-ocr.github.io/tessdoc/Installation.html)

On Debian or Ubuntu, install the terminal package and run `frameguard tui`.

```bash
sudo apt install ./FrameGuard_Linux_x64_Terminal.deb
frameguard tui
```

Use **F2** to set the recording path, **F5** to scan, **F8** to filter by severity, and **E** to export findings. Press **Esc** to cancel an active scan, or **Q** to quit. Pass a wordlist exported from the desktop app with `--rules wordlist.json`.

### Automate a scan

```sh
frameguard scan "demo.mp4" --fps 1 --output findings.json
frameguard scan "demo.mp4" --start 15 --end 20 --fps 5 --text "Client Draft" --format csv
frameguard rules
```

Use `frameguard scan --help` for all options. On Windows, use `.\frameguard.exe` from the extracted folder if the executable is outside PATH.

## How scanning works

FrameGuard samples frames independently of the video's playback rate. FFmpeg decodes the selected frames and Tesseract reads their text on the CPU. Repeated frames can reuse OCR results. Rules group matches by interval and assign Low, Medium, or High severity.

Recordings and OCR results stay on your device. The optional desktop personalization setup fetches a public keyword catalog. Findings exports mask suspected secrets by default. OCR coverage depends on text readability and the selected sampling rate. Asset names are matched when the names or labels are visible in the video.

## Build from source

The [main branch](https://github.com/motionedits246-max/FrameGuard/tree/main) contains the Rust scanner, terminal app, and Tauri desktop frontend. Install stable Rust, the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for the desktop build, and FFmpeg and Tesseract for local scans.

```sh
cargo test --locked
cargo build --locked --release -p frameguard-cli
cargo run --locked -p frameguard-desktop --features custom-protocol
```

On Windows, run `./desktop/scripts/prepare-windows-resources.ps1` before building the desktop app. Use the [issues page](https://github.com/motionedits246-max/FrameGuard/issues) to report a problem.
