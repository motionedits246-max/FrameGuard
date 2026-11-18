#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod dialogs;
mod preview;
mod profile;
mod runtime;

use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use base64::{engine::general_purpose::STANDARD, Engine};
use frameguard_core::{Progress, Rule, ScanOptions, ScanReport, Video, VideoMetadata};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

#[derive(Default)]
struct Session {
    generation: u64,
    selected: Option<Video>,
    active: Option<ActiveScan>,
    clip: Option<preview::ClipRequest>,
}

struct ActiveScan {
    id: String,
    cancelled: Arc<AtomicBool>,
}

type DesktopState = Mutex<Session>;

#[derive(Serialize)]
struct OpenedVideo {
    generation: u64,
    path: String,
    #[serde(flatten)]
    metadata: VideoMetadata,
}

#[derive(Clone, Serialize)]
struct ScanUpdate {
    scan_id: String,
    progress: Progress,
}

fn failure(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

#[tauri::command]
async fn open_recording(app: AppHandle) -> Result<Option<OpenedVideo>, String> {
    let tools = runtime::tools(&app)?;
    let Some(path) = dialogs::choose_file(&app, None).await? else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let video = frameguard_core::open_video_with_tools(path, tools).map_err(failure)?;
        let state = app.state::<DesktopState>();
        let mut session = state.lock().map_err(failure)?;
        if session.active.is_some() {
            return Err("Finish or cancel the current scan before opening a recording.".into());
        }
        app.asset_protocol_scope()
            .allow_file(video.path())
            .map_err(failure)?;
        preview::clear_clip(&app, &mut session);
        session.generation += 1;
        let selected = OpenedVideo {
            generation: session.generation,
            path: video.path().to_string_lossy().into_owned(),
            metadata: video.metadata.clone(),
        };
        session.selected = Some(video);
        Ok(Some(selected))
    })
    .await
    .map_err(failure)?
}

#[tauri::command]
async fn scan_recording(
    app: AppHandle,
    generation: u64,
    scan_id: String,
    options: ScanOptions,
) -> Result<ScanReport, String> {
    runtime::tools(&app)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let video = {
        let state = app.state::<DesktopState>();
        let mut session = state.lock().map_err(failure)?;
        if session.generation != generation {
            return Err("Open the recording again to refresh the selection.".into());
        }
        if session.active.is_some() {
            return Err("A scan is already running.".into());
        }
        let video = session.selected.clone().ok_or("Open an MP4 to scan.")?;
        session.active = Some(ActiveScan {
            id: scan_id.clone(),
            cancelled: Arc::clone(&cancelled),
        });
        video
    };
    let sender = app.clone();
    let event_id = scan_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        frameguard_core::scan(&video, &options, &cancelled, |progress| {
            let _ = sender.emit_to(
                "main",
                "scan-progress",
                ScanUpdate {
                    scan_id: event_id.clone(),
                    progress,
                },
            );
        })
        .map_err(failure)
    })
    .await;
    let state = app.state::<DesktopState>();
    let mut session = state.lock().map_err(failure)?;
    if session
        .active
        .as_ref()
        .is_some_and(|active| active.id == scan_id)
    {
        session.active = None;
    }
    result.map_err(failure)?
}

#[tauri::command]
fn cancel_scan(app: AppHandle, scan_id: String) -> Result<bool, String> {
    let state = app.state::<DesktopState>();
    let session = state.lock().map_err(failure)?;
    let Some(active) = session
        .active
        .as_ref()
        .filter(|active| active.id == scan_id)
    else {
        return Ok(false);
    };
    active.cancelled.store(true, Ordering::Relaxed);
    Ok(true)
}

#[tauri::command]
async fn inspect_frame(app: AppHandle, generation: u64, seconds: f64) -> Result<String, String> {
    runtime::tools(&app)?;
    let video = {
        let state = app.state::<DesktopState>();
        let session = state.lock().map_err(failure)?;
        if session.generation != generation {
            return Err("The selected video has changed.".into());
        }
        session
            .selected
            .clone()
            .ok_or("Open an MP4 to inspect a frame.")?
    };
    tauri::async_runtime::spawn_blocking(move || {
        frameguard_core::capture_frame(&video, seconds)
            .map(|bytes| format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
            .map_err(failure)
    })
    .await
    .map_err(failure)?
}

#[tauri::command]
fn default_wordlist() -> Vec<Rule> {
    frameguard_core::default_rules()
}

#[tauri::command]
fn test_wordlist_rule(rule: Rule, sample: String) -> Result<Vec<[usize; 2]>, String> {
    frameguard_core::test_rule(&rule, &sample)
        .map(|hits| {
            hits.into_iter()
                .map(|[start, end]| {
                    [
                        sample[..start].encode_utf16().count(),
                        sample[..end].encode_utf16().count(),
                    ]
                })
                .collect()
        })
        .map_err(failure)
}

#[tauri::command]
async fn save_export(app: AppHandle, filename: String, contents: String) -> Result<bool, String> {
    let extension = filename.rsplit('.').next().unwrap_or_default();
    if !matches!(extension, "json" | "csv") {
        return Err("Choose JSON or CSV for the export.".into());
    }
    let suggested = if filename == "wordlist.json" {
        "wordlist.json"
    } else if extension == "csv" {
        "frameguard-findings.csv"
    } else {
        "frameguard-findings.json"
    };
    let Some(target) = dialogs::choose_file(&app, Some((suggested, extension))).await? else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let extension = filename.rsplit('.').next().unwrap_or_default();
        if target
            .extension()
            .and_then(|part| part.to_str())
            .is_none_or(|part| !part.eq_ignore_ascii_case(extension))
        {
            return Err(format!("Save the export with a .{extension} extension."));
        }
        let parent = target.parent().ok_or("Choose an export folder.")?;
        let mut pending = tempfile::NamedTempFile::new_in(parent).map_err(failure)?;
        pending.write_all(contents.as_bytes()).map_err(failure)?;
        pending.persist(target).map_err(failure)?;
        Ok(true)
    })
    .await
    .map_err(failure)?
}

fn main() {
    tauri::Builder::default()
        .manage(DesktopState::default())
        .manage(runtime::RuntimeState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            runtime::runtime_status,
            runtime::install_runtime,
            profile::profile_keywords,
            preview::open_finding_clip,
            preview::close_finding_clip,
            open_recording,
            scan_recording,
            cancel_scan,
            inspect_frame,
            default_wordlist,
            test_wordlist_rule,
            save_export,
        ])
        .build(tauri::generate_context!())
        .expect("Starting FrameGuard")
        .run(|app, event| {
            if matches!(event, RunEvent::ExitRequested { .. }) {
                let state = app.state::<DesktopState>();
                if let Ok(mut session) = state.lock() {
                    if let Some(active) = &session.active {
                        active.cancelled.store(true, Ordering::Relaxed);
                    }
                    preview::clear_clip(app, &mut session);
                };
            }
        });
}
