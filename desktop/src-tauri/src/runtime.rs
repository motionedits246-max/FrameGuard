use std::sync::Mutex;

use frameguard_core::ScanTools;
use serde::Serialize;
use tauri::{AppHandle, Manager};

#[cfg(windows)]
mod windows;

#[derive(Default)]
pub struct RuntimeState(Mutex<Option<ScanTools>>);

#[derive(Serialize)]
pub struct RuntimeStatus {
    ready: bool,
    can_install: bool,
    detail: Option<String>,
}

fn status(app: &AppHandle, saved: &mut Option<ScanTools>) -> Result<RuntimeStatus, String> {
    let tools = configured_tools(app)?;
    if cfg!(windows) && !tools.has_managed_files() {
        *saved = None;
        return Ok(RuntimeStatus {
            ready: false,
            can_install: true,
            detail: None,
        });
    }
    let result = tools.verify().map_err(|error| format!("{error:#}"));
    let ready = result.is_ok();
    *saved = ready.then_some(tools);
    Ok(RuntimeStatus {
        ready,
        can_install: cfg!(windows),
        detail: result.err(),
    })
}

#[cfg(windows)]
fn configured_tools(app: &AppHandle) -> Result<ScanTools, String> {
    let root = app.path().app_local_data_dir().map_err(super::failure)?;
    Ok(ScanTools::installed(&root))
}

#[cfg(not(windows))]
fn configured_tools(_: &AppHandle) -> Result<ScanTools, String> {
    Ok(ScanTools::system())
}

pub fn tools(app: &AppHandle) -> Result<ScanTools, String> {
    app.state::<RuntimeState>()
        .0
        .try_lock()
        .map_err(|_| "FrameGuard is checking the scanner components.")?
        .clone()
        .ok_or_else(|| "Complete the FrameGuard scanner setup first.".into())
}

#[tauri::command]
pub async fn runtime_status(app: AppHandle) -> Result<RuntimeStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<RuntimeState>();
        let mut saved = state.0.try_lock().map_err(super::failure)?;
        status(&app, &mut saved)
    })
    .await
    .map_err(super::failure)?
}

#[tauri::command]
pub async fn install_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<RuntimeState>();
        let mut saved = state.0.try_lock().map_err(super::failure)?;
        #[cfg(windows)]
        windows::install_for_app(&app).map_err(super::failure)?;
        status(&app, &mut saved)
    })
    .await
    .map_err(super::failure)?
}
