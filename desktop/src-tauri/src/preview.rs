use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tauri::{AppHandle, Manager};

use crate::{failure, runtime, DesktopState, Session};

pub(super) struct ClipRequest {
    id: u64,
    cancelled: Arc<AtomicBool>,
    file: Option<ClipFile>,
}

struct ClipFile {
    #[cfg(target_os = "linux")]
    _source: Option<frameguard_playback::Media>,
    path: PathBuf,
    _directory: tempfile::TempDir,
}

pub(super) fn clear_clip(app: &AppHandle, session: &mut Session) {
    if let Some(clip) = session.clip.take() {
        clip.cancelled.store(true, Ordering::Relaxed);
        if let Some(file) = clip.file {
            let _ = app.asset_protocol_scope().forbid_file(&file.path);
        }
    }
}

#[tauri::command]
pub async fn open_finding_clip(
    app: AppHandle,
    generation: u64,
    clip_id: u64,
    start: f64,
    end: f64,
) -> Result<Option<String>, String> {
    runtime::tools(&app)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let video = {
        let state = app.state::<DesktopState>();
        let mut session = state.lock().map_err(failure)?;
        if session.generation != generation {
            return Err("The selected video has changed.".into());
        }
        let video = session
            .selected
            .clone()
            .ok_or("Open an MP4 to play a finding.")?;
        clear_clip(&app, &mut session);
        session.clip = Some(ClipRequest {
            id: clip_id,
            cancelled: Arc::clone(&cancelled),
            file: None,
        });
        video
    };

    let prepared =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<ClipFile>, String> {
            let directory = tempfile::Builder::new()
                .prefix("FrameGuard-clip-")
                .tempdir()
                .map_err(failure)?;
            let path = directory.path().join("finding.mp4");
            if !frameguard_core::capture_clip(&video, start, end, &path, &cancelled)
                .map_err(failure)?
            {
                return Ok(None);
            }
            Ok(Some(ClipFile {
                #[cfg(target_os = "linux")]
                _source: None,
                path,
                _directory: directory,
            }))
        })
        .await;

    let state = app.state::<DesktopState>();
    let mut session = state.lock().map_err(failure)?;
    let Some(clip) = session.clip.as_mut().filter(|clip| clip.id == clip_id) else {
        return Ok(None);
    };
    let Some(file) = prepared.map_err(failure)?? else {
        return Ok(None);
    };
    #[cfg(target_os = "linux")]
    let mut file = file;
    app.asset_protocol_scope()
        .allow_file(&file.path)
        .map_err(failure)?;
    #[cfg(not(target_os = "linux"))]
    let path = file.path.to_string_lossy().into_owned();
    #[cfg(target_os = "linux")]
    let path = {
        let source = crate::playback::open(&app, &file.path)?;
        let url = source.url().to_owned();
        file._source = Some(source);
        url
    };
    clip.file = Some(file);
    Ok(Some(path))
}

#[tauri::command]
pub fn close_finding_clip(app: AppHandle, clip_id: u64) -> Result<(), String> {
    let state = app.state::<DesktopState>();
    let mut session = state.lock().map_err(failure)?;
    if session.clip.as_ref().is_some_and(|clip| clip.id == clip_id) {
        clear_clip(&app, &mut session);
    }
    Ok(())
}
