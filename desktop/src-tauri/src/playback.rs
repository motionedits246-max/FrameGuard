use std::path::Path;

use frameguard_playback::{Media, Server};
use tauri::{AppHandle, Manager};

use crate::failure;

pub struct PlaybackState(Result<Server, String>);

pub fn initialize(app: &tauri::App) {
    let server = tauri::async_runtime::block_on(Server::bind())
        .map_err(|error| format!("Starting local video playback: {error}"));
    app.manage(PlaybackState(server));
}

pub fn open(app: &AppHandle, path: &Path) -> Result<Media, String> {
    app.state::<PlaybackState>()
        .0
        .as_ref()
        .map_err(Clone::clone)?
        .open(path)
        .map_err(failure)
}
