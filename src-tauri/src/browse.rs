use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, WebviewWindow};

use crate::file_dialog;
use crate::formats;
use crate::resources;

/// The files a browser window offers plus the index of the file to open.
///
/// A picker that the user cancelled returns `None`; every other command returns
/// a selection whose `index` always addresses `paths`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseSelection {
    pub paths: Vec<String>,
    pub index: usize,
}

/// Picks one or more schematics. A single file opens at its alphabetical place
/// among the supported files in the same directory; several files open as the
/// selected subset in alphabetical order.
#[tauri::command]
pub async fn choose_files(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<BrowseSelection>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let extensions: Vec<_> = formats::EXTENSIONS
            .iter()
            .map(|extension| &extension[1..])
            .collect();
        let Some(paths) = file_dialog::pick_files(
            &app,
            window,
            "Open Minecraft schematics",
            "Minecraft schematics",
            &extensions,
            true,
        )?
        else {
            return Ok(None);
        };
        selection_for_picked(paths).map(Some)
    })
    .await
    .map_err(|error| format!("Unable to show the file picker: {error}"))?
}

/// Picks a directory and opens the first supported schematic directly inside it.
#[tauri::command]
pub async fn choose_folder(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Option<BrowseSelection>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(directory) = file_dialog::pick_folder(&app, window, "Open schematic folder")?
        else {
            return Ok(None);
        };
        selection_for_directory(&directory).map(Some)
    })
    .await
    .map_err(|error| format!("Unable to show the folder picker: {error}"))?
}

/// Reopens a known path with its supported neighbours, or lists a directory.
#[tauri::command]
pub async fn browse_siblings(path: String) -> Result<BrowseSelection, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("Unable to open {}: {error}", path.display()))?;
        if metadata.is_dir() {
            selection_for_directory(&path)
        } else if metadata.is_file() {
            selection_for_file(path)
        } else {
            Err(format!(
                "{} is not a regular file or folder.",
                path.display()
            ))
        }
    })
    .await
    .map_err(|error| format!("Unable to browse nearby schematics: {error}"))?
}

/// Opens a dropped set of paths. Unsupported paths, folders, and missing files
/// are ignored; a set without a supported regular file opens nothing.
#[tauri::command]
pub async fn browse_paths(paths: Vec<String>) -> Result<Option<BrowseSelection>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = supported_dropped_files(paths);
        if files.is_empty() {
            return Ok(None);
        }
        selection_for_picked(files).map(Some)
    })
    .await
    .map_err(|error| format!("Unable to browse the dropped schematics: {error}"))?
}

/// Applies the shared single/multiple-file rule to an already-resolved list.
fn selection_for_picked(paths: Vec<PathBuf>) -> Result<BrowseSelection, String> {
    let mut paths = unique_paths(paths);
    if paths.is_empty() {
        return Err("No schematic files were selected.".into());
    }
    if paths.len() == 1 {
        let path = paths.remove(0);
        return selection_for_file(path);
    }
    selection_for_subset(paths)
}

/// Multiple picks stay a subset; they never pull in unselected neighbours.
fn selection_for_subset(mut paths: Vec<PathBuf>) -> Result<BrowseSelection, String> {
    sort_paths(&mut paths);
    let paths = paths
        .iter()
        .map(|path| resources::path_string(path))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BrowseSelection { paths, index: 0 })
}

/// Opens a file at its sorted position and includes its supported siblings.
fn selection_for_file(path: PathBuf) -> Result<BrowseSelection, String> {
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Unable to open {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a file.", path.display()));
    }
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // `read_dir(".")` prefixes entries with "./"; match that spelling so a bare
    // relative path is not added twice while its explicit spelling is preserved.
    let explicit_for_match = if path.is_relative()
        && path
            .parent()
            .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        directory.join(&path)
    } else {
        path.clone()
    };
    let mut siblings = supported_files(directory)?;
    if let Some(index) = siblings
        .iter()
        .position(|candidate| same_path(candidate, &explicit_for_match))
    {
        siblings[index] = path.clone();
    } else {
        siblings.push(path.clone());
    }
    sort_paths(&mut siblings);
    let index = siblings
        .iter()
        .position(|candidate| {
            same_path(candidate, &path) || same_path(candidate, &explicit_for_match)
        })
        .ok_or_else(|| format!("Unable to locate {} among its siblings.", path.display()))?;
    let paths = siblings
        .iter()
        .map(|sibling| resources::path_string(sibling))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BrowseSelection { paths, index })
}

/// Lists the supported regular files directly inside a directory, never recursively.
fn selection_for_directory(directory: &Path) -> Result<BrowseSelection, String> {
    let mut files = supported_files(directory)?;
    sort_paths(&mut files);
    if files.is_empty() {
        return Err(format!(
            "No supported schematics were found directly in {}. Supported formats: {}.",
            directory.display(),
            formats::EXTENSIONS.join(", ")
        ));
    }
    let paths = files
        .iter()
        .map(|file| resources::path_string(file))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BrowseSelection { paths, index: 0 })
}

fn supported_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("Unable to read {}: {error}", directory.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("Unable to read {}: {error}", directory.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Unable to inspect {}: {error}", entry.path().display()))?;
        if file_type.is_file() && formats::is_supported_path(&entry.path()) {
            files.push(entry.path());
        }
    }
    Ok(files)
}

fn supported_dropped_files(paths: Vec<String>) -> Vec<PathBuf> {
    unique_paths(paths.into_iter().map(PathBuf::from).collect())
        .into_iter()
        .filter(|path| formats::is_supported_path(path) && path.is_file())
        .collect()
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::with_capacity(paths.len());
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// Case-insensitive by filename, with the exact name and native path breaking ties.
fn sort_paths(paths: &mut [PathBuf]) {
    paths.sort_by_cached_key(|path| {
        let name = path
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_default();
        let folded = name.to_string_lossy().to_lowercase();
        (folded, name, path.clone())
    });
}

/// Windows compares paths case-insensitively; other platforms keep exact names.
fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left == right
            || left
                .to_string_lossy()
                .eq_ignore_ascii_case(right.to_string_lossy().as_ref())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "litematica-preview-browse-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn write(&self, name: &str) -> PathBuf {
            let path = self.path.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, []).unwrap();
            path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn strings(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| resources::path_string(path).unwrap())
            .collect()
    }

    #[test]
    fn explicit_file_keeps_its_sorted_index_and_spelling() {
        let directory = TempDirectory::new();
        let alpha = directory.write("alpha.schem");
        let bravo = directory.write("bravo.SCHEM");
        let charlie = directory.write("charlie.litematic");

        let selection = selection_for_file(bravo.clone()).unwrap();

        assert_eq!(selection.paths, strings(&[alpha, bravo.clone(), charlie]));
        assert_eq!(selection.index, 1);
        assert_eq!(selection.paths[1], resources::path_string(&bravo).unwrap());
    }

    #[test]
    fn directory_listing_ignores_nested_and_unsupported_entries() {
        let directory = TempDirectory::new();
        let alpha = directory.write("Alpha.SCHEM");
        let beta = directory.write("beta.litematic");
        let zeta = directory.write("zeta.schem");
        directory.write("notes.txt");
        directory.write("nested/inside.schem");

        let selection = selection_for_directory(&directory.path).unwrap();

        assert_eq!(selection.paths, strings(&[alpha, beta, zeta]));
        assert_eq!(selection.index, 0);
    }

    #[test]
    fn sort_is_case_insensitive_with_deterministic_ties() {
        let mut paths = ["b.schem", "a.schem", "B.schem", "A.schem"]
            .map(PathBuf::from)
            .to_vec();

        sort_paths(&mut paths);
        let sorted = paths.clone();
        sort_paths(&mut paths);

        assert_eq!(
            paths,
            ["A.schem", "a.schem", "B.schem", "b.schem"]
                .map(PathBuf::from)
                .to_vec()
        );
        assert_eq!(paths, sorted);
    }

    #[test]
    fn multiple_selection_sorts_by_filename_across_directories() {
        let directory = TempDirectory::new();
        let alpha = directory.write("z-folder/alpha.schem");
        let zeta = directory.write("a-folder/zeta.schem");

        let selection = selection_for_picked(vec![zeta.clone(), alpha.clone()]).unwrap();

        assert_eq!(selection.paths, strings(&[alpha, zeta]));
        assert_eq!(selection.index, 0);
    }

    #[test]
    fn multiple_selection_does_not_expand_siblings() {
        let directory = TempDirectory::new();
        let alpha = directory.write("alpha.schem");
        let bravo = directory.write("bravo.schem");
        directory.write("charlie.schem");

        let selection = selection_for_picked(vec![bravo.clone(), alpha.clone()]).unwrap();

        assert_eq!(selection.paths, strings(&[alpha, bravo]));
        assert_eq!(selection.index, 0);
        assert!(
            !selection
                .paths
                .iter()
                .any(|path| path.ends_with("charlie.schem"))
        );
    }
}
