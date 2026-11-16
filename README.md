# FrameGuard

FrameGuard finds sensitive text in local screen recordings before you publish or share them. It samples an MP4, reads the selected frames with CPU OCR, and reports where potentially exposed text appeared. The Windows desktop app, terminal, and CLI share one Rust scanner.

## What you can do

- Sample once per second by default. Set a custom rate or scan a time range to inspect brief exposures.
- Flag custom text and edit a reusable wordlist. Rules support plain text, regular expressions, and Low, Medium, or High risk.
- Review timestamped findings and the matching image region. The desktop app can rescan near a finding at 5 FPS.
- Export JSON or CSV with suspected secret values masked by default.
- Scan from a terminal, view progress, cancel a scan, and open the sampled frame for a finding.

## Windows setup

Install stable Rust and the Tauri Windows prerequisites, including the C++ build tools and WebView2. [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)

Install FFmpeg and Tesseract with English OCR data. Put `ffmpeg`, `ffprobe`, and `tesseract` on your PATH. You can also point `FRAMEGUARD_FFMPEG`, `FRAMEGUARD_FFPROBE`, and `FRAMEGUARD_TESSERACT` at the corresponding executables.

Check the tools from PowerShell.

```powershell
ffmpeg -version
ffprobe -version
tesseract --list-langs
```

Run the desktop interface from the repository root.

```powershell
cargo run --locked -p frameguard-desktop --features custom-protocol
```

## CLI and terminal

The following commands run from the repository root and work wherever the Rust and OCR prerequisites are installed.

```powershell
cargo run --locked -p frameguard-cli -- scan "C:\Videos\demo.mp4" --fps 1 --format json --output findings.json
cargo run --locked -p frameguard-cli -- scan "C:\Videos\demo.mp4" --fps 5 --start 15 --end 18 --text "client.example"
cargo run --locked -p frameguard-cli -- tui "C:\Videos\demo.mp4"
```

Use **Export wordlist** in the desktop Rules view to share custom rules with the CLI or TUI. Pass the exported file with `--rules wordlist.json`. The default list is also available from `frameguard rules`. Use `frameguard scan --help` for all flags.

The terminal footer lists its keyboard shortcuts. Use **F2** to set a video path and **F5** to start scanning. **Enter** opens a sampled frame, **E** exports findings, and **Esc** cancels the active scan.

## Development

- `crates/frameguard-core` reads video metadata, samples frames using FFmpeg, runs Tesseract, groups consecutive hits, and writes reports.
- `crates/frameguard-cli` provides the `frameguard` command and terminal interface.
- `desktop/src-tauri` provides native dialogs, scan progress events, cancellation, and scoped access to selected recordings.
- `desktop/ui` contains the approved interface, with local Lucide SVG icons and no frontend build step.

```powershell
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
```

Sampling and OCR can miss text, especially when it is small or appears between frames. Review the footage before sharing it. Video blur export, packaged OCR dependencies, and signed releases are future work.
