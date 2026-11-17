# FrameGuard

FrameGuard finds sensitive text in local screen recordings before you publish or share them. It samples an MP4, reads the selected frames with CPU OCR, and reports where potentially exposed text appeared. The Windows desktop app, terminal, and CLI share one Rust scanner.

## What you can do

- Sample once per second by default. Set a custom rate or scan a time range to inspect brief exposures.
- Flag custom text and edit a reusable wordlist. Rules support plain text, regular expressions, and Low, Medium, or High risk.
- Review timestamped findings and the matching image region. The desktop app can rescan near a finding at 5 FPS.
- Export JSON or CSV with suspected secret values masked by default.
- Scan from a terminal, view progress, cancel a scan, and open the sampled frame for a finding.

## Windows setup

Run the Windows x64 FrameGuard setup EXE and launch the app. In **Set up local scanning**, select **Download & install**. FrameGuard downloads about 66 MB of scanner components and opens the dashboard after checking video decoding and English OCR.

The components live under your Windows user profile. Scans work offline after setup. The Windows CLI and TUI can use the same components.

## CLI and terminal

The following commands run from the repository root. If you use the standalone CLI without completing desktop setup, install FFmpeg, ffprobe, and Tesseract with the desired language data. The CLI also accepts `FRAMEGUARD_FFMPEG`, `FRAMEGUARD_FFPROBE`, and `FRAMEGUARD_TESSERACT` executable overrides.

```powershell
cargo run --locked -p frameguard-cli -- scan "C:\Videos\demo.mp4" --fps 1 --format json --output findings.json
cargo run --locked -p frameguard-cli -- scan "C:\Videos\demo.mp4" --fps 5 --start 15 --end 18 --text "client.example"
cargo run --locked -p frameguard-cli -- tui "C:\Videos\demo.mp4"
```

Use **Export wordlist** in the desktop Rules view to share custom rules with the CLI or TUI. Pass the exported file with `--rules wordlist.json`. The default list is also available from `frameguard rules`. Use `frameguard scan --help` for all flags.

The terminal footer lists its keyboard shortcuts. Use **F2** to set a video path and **F5** to start scanning. **Enter** opens a sampled frame, **E** exports findings, and **Esc** cancels the active scan.

## Development

To build the Windows desktop app from source, install stable Rust, 7-Zip and the [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/), then run:

```powershell
./desktop/scripts/prepare-windows-resources.ps1
cargo run --locked -p frameguard-desktop --features custom-protocol
```

- `crates/frameguard-core` reads video metadata, samples frames using FFmpeg, runs Tesseract, groups consecutive hits, and writes reports.
- `crates/frameguard-cli` provides the `frameguard` command and terminal interface.
- `desktop/src-tauri` provides native dialogs, scan progress events, cancellation, and scoped access to selected recordings.
- `desktop/ui` contains the approved interface, with local Lucide SVG icons and no frontend build step.

```powershell
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
```

Sampling and OCR can miss text, especially when it is small or appears between frames. Review the footage before sharing it. Video blur export and signed releases are future work.
