use std::path::PathBuf;

/// Picks one or more files with the platform's native multi-select dialog.
///
/// Must run on a blocking worker, never the window's event-loop thread: Cocoa
/// deadlocks when its main thread blocks inside the dialog, while the Linux
/// portal and Windows shell dialogs block the polling thread until the user
/// answers. A cancelled dialog, or a desktop whose file chooser cannot open,
/// returns an empty list so the caller opens nothing.
pub fn pick_files(title: &str, filter_name: &str, extensions: &[&str]) -> Vec<PathBuf> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(title)
        .add_filter(filter_name, extensions);
    pollster::block_on(dialog.pick_files())
        .map(|files| {
            files
                .into_iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        })
        .unwrap_or_default()
}
