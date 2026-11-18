use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::failure;

pub async fn choose_file(
    app: &AppHandle,
    export: Option<(&str, &str)>,
) -> Result<Option<PathBuf>, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    let export = export.map(|(name, extension)| (name.to_owned(), extension.to_owned()));
    app.run_on_main_thread(move || {
        let mut dialog = handle.dialog().file();
        if let Some(parent) = handle.get_webview_window("main") {
            dialog = dialog.set_parent(&parent);
        }
        match export {
            Some((name, extension)) => {
                dialog
                    .set_title("Save FrameGuard export")
                    .set_file_name(name)
                    .add_filter("FrameGuard export", &[&extension])
                    .save_file(move |file| {
                        let _ = send.send(file);
                    });
            }
            None => {
                dialog
                    .set_title("Open a recording")
                    .add_filter("MP4 video", &["mp4"])
                    .pick_file(move |file| {
                        let _ = send.send(file);
                    });
            }
        }
    })
    .map_err(failure)?;
    receive
        .await
        .map_err(|_| "The file picker closed unexpectedly. Try opening it again.".to_owned())?
        .map(|file| file.into_path().map_err(failure))
        .transpose()
}
