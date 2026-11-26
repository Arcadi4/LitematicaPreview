use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::formats;

/// The files a browser offers plus the index of the file to open.
///
/// Paths stay native, so a filename that is not UTF-8 keeps working, and
/// `index` always addresses `paths`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowseSelection {
    pub paths: Vec<PathBuf>,
    pub index: usize,
}

/// Applies the single/multiple-file rule to paths the user picked. A single
/// file opens at its alphabetical place among the supported files in the same
/// directory; several files open as the selected subset in alphabetical order.
/// Unsupported paths, folders, and missing files are ignored.
pub fn selection_for_picked(paths: Vec<PathBuf>) -> Result<BrowseSelection, String> {
    let mut paths = supported_paths(paths);
    if paths.is_empty() {
        return Err("No supported schematic files were selected.".into());
    }
    if paths.len() == 1 {
        let path = paths.remove(0);
        return selection_for_file(path);
    }
    Ok(selection_for_subset(paths))
}

/// Multiple picks stay a subset; they never pull in unselected neighbours.
fn selection_for_subset(mut paths: Vec<PathBuf>) -> BrowseSelection {
    sort_paths(&mut paths);
    BrowseSelection { paths, index: 0 }
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
    Ok(BrowseSelection {
        paths: siblings,
        index,
    })
}

/// Lists the supported regular files directly inside a directory, never recursively.
pub fn selection_for_directory(directory: &Path) -> Result<BrowseSelection, String> {
    let mut files = supported_files(directory)?;
    sort_paths(&mut files);
    if files.is_empty() {
        return Err(format!(
            "No supported schematics were found directly in {}. Supported formats: {}.",
            directory.display(),
            formats::EXTENSIONS.join(", ")
        ));
    }
    Ok(BrowseSelection {
        paths: files,
        index: 0,
    })
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

/// Keeps the existing regular files with a supported extension, without
/// duplicates and in the order given. Folders, missing files, and unsupported
/// paths are dropped.
pub fn supported_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    unique_paths(paths)
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

        fn write(&self, name: impl AsRef<Path>) -> PathBuf {
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

    #[test]
    fn explicit_file_keeps_its_sorted_index_and_spelling() {
        let directory = TempDirectory::new();
        let alpha = directory.write("alpha.schem");
        let bravo = directory.write("bravo.SCHEM");
        let charlie = directory.write("charlie.litematic");

        let selection = selection_for_file(bravo.clone()).unwrap();

        assert_eq!(selection.paths, [alpha, bravo.clone(), charlie]);
        assert_eq!(selection.index, 1);
        assert_eq!(selection.paths[1], bravo);
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

        assert_eq!(selection.paths, [alpha, beta, zeta]);
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

        assert_eq!(selection.paths, [alpha, zeta]);
        assert_eq!(selection.index, 0);
    }

    #[test]
    fn multiple_selection_does_not_expand_siblings() {
        let directory = TempDirectory::new();
        let alpha = directory.write("alpha.schem");
        let bravo = directory.write("bravo.schem");
        directory.write("charlie.schem");

        let selection = selection_for_picked(vec![bravo.clone(), alpha.clone()]).unwrap();

        assert_eq!(selection.paths, [alpha, bravo]);
        assert_eq!(selection.index, 0);
        assert!(
            !selection
                .paths
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == "charlie.schem"))
        );
    }

    #[test]
    fn unsupported_picks_are_dropped() {
        let directory = TempDirectory::new();
        let alpha = directory.write("alpha.schem");
        let bravo = directory.write("bravo.schem");
        let notes = directory.write("notes.txt");

        let selection = selection_for_picked(vec![notes, alpha.clone()]).unwrap();

        assert_eq!(selection.paths, [alpha, bravo]);
        assert_eq!(selection.index, 0);
        assert!(selection_for_picked(Vec::new()).is_err());
    }

    /// APFS rejects filenames that are not UTF-8, so listing coverage runs only
    /// where the filesystem accepts them.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_filename_that_is_not_utf8_is_listed() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let directory = TempDirectory::new();
        directory.write("alpha.schem");
        let path = directory.write(std::ffi::OsString::from_vec(b"caf\xe9.schem".to_vec()));

        let selection = selection_for_directory(&directory.path).unwrap();

        assert_eq!(selection.paths.len(), 2);
        assert_eq!(
            selection.paths[1].file_name().unwrap().as_bytes(),
            b"caf\xe9.schem"
        );
        assert_eq!(selection.paths[1], path);
    }

    /// The browser never converts paths to UTF-8, so an in-memory name that is
    /// not UTF-8 keeps its exact bytes through sorting.
    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_survives_sorting() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let mut paths = vec![
            PathBuf::from(std::ffi::OsString::from_vec(b"zeta\xe9.schem".to_vec())),
            PathBuf::from("alpha.schem"),
        ];

        sort_paths(&mut paths);

        assert_eq!(paths[0], PathBuf::from("alpha.schem"));
        assert_eq!(paths[1].as_os_str().as_bytes(), b"zeta\xe9.schem");
    }

    #[test]
    fn supported_paths_keep_existing_supported_files_once() {
        let directory = TempDirectory::new();
        let alpha = directory.write("alpha.schem");
        let notes = directory.write("notes.txt");
        let folder = directory.path.join("folder.schem");
        std::fs::create_dir_all(&folder).unwrap();
        let missing = directory.path.join("missing.schem");

        let filtered = supported_paths(vec![alpha.clone(), notes, alpha.clone(), folder, missing]);

        assert_eq!(filtered, [alpha]);
    }
}
