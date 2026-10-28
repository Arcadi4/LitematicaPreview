use std::path::PathBuf;

use tauri::{AppHandle, WebviewWindow};

/// Must run on a blocking worker, not the native event-loop thread.
pub fn pick_files(
    app: &AppHandle,
    window: WebviewWindow,
    title: &'static str,
    filter_name: &'static str,
    extensions: &[&str],
    multiple: bool,
) -> Result<Option<Vec<PathBuf>>, String> {
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;

        let extensions: Vec<String> = extensions.iter().map(|value| (*value).into()).collect();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let parent = match window.gtk_window() {
                Ok(parent) => parent,
                Err(error) => {
                    let _ = sender.send(Err(format!("Unable to parent the file picker: {error}")));
                    return;
                }
            };
            let dialog = gtk::FileChooserNative::builder()
                .title(title)
                .transient_for(&parent)
                .modal(true)
                .action(gtk::FileChooserAction::Open)
                .select_multiple(multiple)
                .build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(filter_name));
            for extension in extensions {
                filter.add_pattern(&format!("*.{extension}"));
            }
            dialog.add_filter(filter);
            let all_files = gtk::FileFilter::new();
            all_files.set_name(Some("All files"));
            all_files.add_pattern("*");
            dialog.add_filter(all_files);
            gtk::glib::MainContext::default().spawn_local(async move {
                let response = dialog.run_future().await;
                let files = (response == gtk::ResponseType::Accept).then(|| dialog.filenames());
                dialog.destroy();
                let _ = sender.send(Ok(files));
            });
        })
        .map_err(|error| format!("Unable to show the file picker: {error}"))?;
        receiver
            .recv()
            .map_err(|error| format!("The file picker closed unexpectedly: {error}"))?
    }
    #[cfg(not(target_os = "linux"))]
    {
        use tauri_plugin_dialog::DialogExt;

        let dialog = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(title)
            .add_filter(filter_name, extensions)
            .add_filter("All files", &["*"]);
        let files = if multiple {
            dialog.blocking_pick_files()
        } else {
            dialog.blocking_pick_file().map(|file| vec![file])
        };
        files
            .map(|files| {
                files
                    .into_iter()
                    .map(|file| {
                        file.into_path()
                            .map_err(|error| format!("Unable to open the selected file: {error}"))
                    })
                    .collect()
            })
            .transpose()
    }
}

/// Must run on a blocking worker, not the native event-loop thread.
pub fn save_file(
    app: &AppHandle,
    window: WebviewWindow,
    title: &'static str,
    filters: &'static [(&'static str, &'static [&'static str])],
    default_name: &'static str,
) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let parent = match window.gtk_window() {
                Ok(parent) => parent,
                Err(error) => {
                    let _ = sender.send(Err(format!("Unable to parent the save dialog: {error}")));
                    return;
                }
            };
            let dialog = gtk::FileChooserNative::builder()
                .title(title)
                .transient_for(&parent)
                .modal(true)
                .action(gtk::FileChooserAction::Save)
                .build();
            dialog.set_current_name(default_name);
            dialog.set_do_overwrite_confirmation(true);
            for (name, extensions) in filters {
                let filter = gtk::FileFilter::new();
                filter.set_name(Some(*name));
                for extension in *extensions {
                    filter.add_pattern(&format!("*.{extension}"));
                }
                dialog.add_filter(filter);
            }
            gtk::glib::MainContext::default().spawn_local(async move {
                let response = dialog.run_future().await;
                let file = (response == gtk::ResponseType::Accept)
                    .then(|| dialog.filename())
                    .flatten();
                dialog.destroy();
                let _ = sender.send(Ok(file));
            });
        })
        .map_err(|error| format!("Unable to show the save dialog: {error}"))?;
        receiver
            .recv()
            .map_err(|error| format!("The save dialog closed unexpectedly: {error}"))?
    }
    #[cfg(not(target_os = "linux"))]
    {
        use tauri_plugin_dialog::DialogExt;

        let mut dialog = app
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(title)
            .set_file_name(default_name);
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, *extensions);
        }
        dialog
            .blocking_save_file()
            .map(|file| {
                file.into_path()
                    .map_err(|error| format!("Unable to use the selected file: {error}"))
            })
            .transpose()
    }
}
