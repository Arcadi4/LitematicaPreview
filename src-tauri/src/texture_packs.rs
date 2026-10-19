use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexturePack {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexturePackState {
    pub packs: Vec<TexturePack>,
    pub selected: Option<String>,
}

/// A directory of validated resource-pack ZIPs and the selected entry.
/// Every mutation loads and saves the index under one lock.
#[derive(Clone)]
pub struct TexturePackLibrary {
    directory: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl TexturePackLibrary {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn state(&self) -> Result<TexturePackState, String> {
        let _guard = self.lock.lock();
        self.load()
    }

    /// Resolves the selected archive, if any. A missing file is an error so
    /// the preview does not silently render with different textures.
    pub fn selected_archive(&self) -> Result<Option<PathBuf>, String> {
        let _guard = self.lock.lock();
        let state = self.load()?;
        let Some(id) = state.selected.as_deref() else {
            return Ok(None);
        };
        let pack = state
            .packs
            .iter()
            .find(|pack| pack.id == id)
            .ok_or("The selected texture pack is not in the library.")?;
        let archive = self.archive_path(id);
        if !archive.is_file() {
            return Err(format!(
                "The saved copy of “{}” is missing. Import the texture pack again or choose Vanilla.",
                pack.name
            ));
        }
        Ok(Some(archive))
    }

    /// Stores an archive after `validate` accepts its bytes. A rejected or
    /// unsaved archive leaves no entry behind.
    pub fn import(
        &self,
        name: &str,
        bytes: &[u8],
        validate: impl FnOnce(&[u8]) -> Result<(), String>,
    ) -> Result<TexturePack, String> {
        validate(bytes)?;
        let id = Uuid::new_v4().to_string();
        let archive = self.archive_path(&id);
        // Writing the archive can take seconds, so keep the index lock free.
        fs::create_dir_all(&self.directory)
            .map_err(|error| format!("Unable to create the texture pack folder: {error}"))?;
        let temporary = self.directory.join(format!("{id}.zip.tmp"));
        fs::write(&temporary, bytes)
            .map_err(|error| format!("Unable to save the texture pack: {error}"))?;
        if let Err(error) = fs::rename(&temporary, &archive) {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Unable to save the texture pack: {error}"));
        }
        let _guard = self.lock.lock();
        let mut state = match self.load() {
            Ok(state) => state,
            Err(error) => {
                let _ = fs::remove_file(&archive);
                return Err(error);
            }
        };
        let name = unique_name(&state.packs, name);
        state.packs.push(TexturePack {
            id: id.clone(),
            name: name.clone(),
        });
        if let Err(error) = self.save(&state) {
            let _ = fs::remove_file(&archive);
            return Err(error);
        }
        Ok(TexturePack { id, name })
    }

    pub fn select(
        &self,
        id: Option<&str>,
        validate: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<TexturePackState, String> {
        let Some(id) = id else {
            let _guard = self.lock.lock();
            let mut state = self.load()?;
            state.selected = None;
            self.save(&state)?;
            return Ok(state);
        };
        // Parsing a pack can take seconds, so validate before taking the lock.
        let (name, archive) = {
            let _guard = self.lock.lock();
            let state = self.load()?;
            let pack = state
                .packs
                .iter()
                .find(|pack| pack.id == id)
                .ok_or("The selected texture pack is not in the library.")?;
            (pack.name.clone(), self.archive_path(id))
        };
        validate(&archive).map_err(|error| format!("Unable to use “{name}”: {error}"))?;
        let _guard = self.lock.lock();
        let mut state = self.load()?;
        if !state.packs.iter().any(|pack| pack.id == id) {
            return Err("The selected texture pack is not in the library.".into());
        }
        state.selected = Some(id.to_owned());
        self.save(&state)?;
        Ok(state)
    }

    pub fn remove(&self, id: &str) -> Result<TexturePackState, String> {
        let _guard = self.lock.lock();
        let mut state = self.load()?;
        let Some(position) = state.packs.iter().position(|pack| pack.id == id) else {
            return Ok(state);
        };
        state.packs.remove(position);
        if state.selected.as_deref() == Some(id) {
            state.selected = None;
        }
        self.save(&state)?;
        let _ = fs::remove_file(self.archive_path(id));
        Ok(state)
    }

    fn index_path(&self) -> PathBuf {
        self.directory.join("library.json")
    }

    fn archive_path(&self, id: &str) -> PathBuf {
        self.directory.join(format!("{id}.zip"))
    }

    fn load(&self) -> Result<TexturePackState, String> {
        let index = self.index_path();
        let bytes = match fs::read(&index) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TexturePackState::default())
            }
            Err(error) => return Err(format!("Unable to read the texture pack library: {error}")),
        };
        let Ok(mut state) = serde_json::from_slice::<TexturePackState>(&bytes) else {
            let _ = fs::rename(&index, self.directory.join("library.invalid.json"));
            return Ok(TexturePackState::default());
        };
        let mut seen = HashSet::new();
        state
            .packs
            .retain(|pack| Uuid::parse_str(&pack.id).is_ok() && seen.insert(pack.id.clone()));
        if state
            .selected
            .as_ref()
            .is_some_and(|id| !state.packs.iter().any(|pack| &pack.id == id))
        {
            state.selected = None;
        }
        Ok(state)
    }

    fn save(&self, state: &TexturePackState) -> Result<(), String> {
        fs::create_dir_all(&self.directory)
            .map_err(|error| format!("Unable to create the texture pack folder: {error}"))?;
        let data = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("Unable to encode the texture pack library: {error}"))?;
        let temporary = self.directory.join("library.json.tmp");
        fs::write(&temporary, data)
            .map_err(|error| format!("Unable to write the texture pack library: {error}"))?;
        fs::rename(&temporary, self.index_path()).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            format!("Unable to save the texture pack library: {error}")
        })
    }
}

fn unique_name(packs: &[TexturePack], requested: &str) -> String {
    let requested = requested.trim();
    let base = if requested.is_empty() {
        "Texture Pack"
    } else {
        requested
    };
    let mut name = base.to_owned();
    let mut suffix = 2;
    while packs
        .iter()
        .any(|pack| pack.name.to_lowercase() == name.to_lowercase())
    {
        name = format!("{base} ({suffix})");
        suffix += 1;
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "litematica-preview-texture-packs-{}",
                Uuid::new_v4()
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn library(&self) -> TexturePackLibrary {
            TexturePackLibrary::new(self.root.clone())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn imports_store_archives_and_restore_the_selection() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let first = library.import("Cottage", b"first", |_| Ok(())).unwrap();
        let second = library.import("Cottage", b"second", |_| Ok(())).unwrap();
        assert_eq!(first.name, "Cottage");
        assert_eq!(second.name, "Cottage (2)");
        assert!(fixture.root.join(format!("{}.zip", first.id)).is_file());
        assert!(fixture.root.join(format!("{}.zip", second.id)).is_file());

        let selected = library
            .select(Some(&second.id), |path| {
                assert_eq!(path, library.archive_path(&second.id));
                Ok(())
            })
            .unwrap();
        assert_eq!(selected.selected.as_deref(), Some(second.id.as_str()));

        let restored = fixture.library().state().unwrap();
        assert_eq!(restored, selected);
        assert_eq!(restored.packs, [first, second]);
    }

    #[test]
    fn accepts_and_reloads_a_real_resource_pack() {
        use litematica_preview_core::resource_pack_overlay;
        use nucleation::meshing::ResourcePackSource;

        let fixture = Fixture::new();
        let library = fixture.library();
        let bytes = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../Assets/pack.zip"))
            .expect("bundled resource pack");
        let base = ResourcePackSource::from_bytes(&bytes).expect("parse bundled resource pack");
        let pack = library
            .import("Vanilla copy", &bytes, |bytes| {
                resource_pack_overlay(&base, bytes).map(|_| ())
            })
            .expect("import bundled resource pack");

        let selected = library
            .select(Some(&pack.id), |path| {
                let bytes = fs::read(path).expect("saved resource pack");
                resource_pack_overlay(&base, &bytes).map(|_| ())
            })
            .expect("select imported resource pack");
        assert_eq!(selected.selected.as_deref(), Some(pack.id.as_str()));
        assert_eq!(
            library.selected_archive().expect("selected archive"),
            Some(fixture.root.join(format!("{}.zip", pack.id)))
        );
    }

    #[test]
    fn rejected_archives_leave_no_entry_or_file() {
        let fixture = Fixture::new();
        let library = fixture.library();
        assert_eq!(
            library
                .import("Broken", b"broken", |_| Err("Invalid pack".into()))
                .unwrap_err(),
            "Invalid pack"
        );
        assert!(library.state().unwrap().packs.is_empty());
        assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
    }

    #[test]
    fn rejected_selections_keep_the_previous_choice() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let pack = library.import("Pack", b"data", |_| Ok(())).unwrap();
        library.select(Some(&pack.id), |_| Ok(())).unwrap();
        assert!(library
            .select(Some(&pack.id), |_| Err("Unreadable".into()))
            .unwrap_err()
            .contains("Unreadable"));
        assert!(library.select(Some("missing"), |_| Ok(())).is_err());
        assert_eq!(
            library.state().unwrap().selected.as_deref(),
            Some(pack.id.as_str())
        );
    }

    #[test]
    fn removing_the_selected_pack_clears_the_choice_and_archive() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let pack = library.import("Pack", b"data", |_| Ok(())).unwrap();
        library.select(Some(&pack.id), |_| Ok(())).unwrap();

        let state = library.remove(&pack.id).unwrap();
        assert!(state.packs.is_empty());
        assert!(state.selected.is_none());
        assert!(!fixture.root.join(format!("{}.zip", pack.id)).exists());
    }

    #[test]
    fn recovers_from_an_unreadable_index_without_deleting_archives() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let pack = library.import("Pack", b"data", |_| Ok(())).unwrap();
        fs::write(fixture.root.join("library.json"), b"not json").unwrap();

        assert!(library.state().unwrap().packs.is_empty());
        assert!(fixture.root.join("library.invalid.json").is_file());
        assert!(fixture.root.join(format!("{}.zip", pack.id)).is_file());
    }

    #[test]
    fn ignores_index_entries_with_path_like_ids() {
        let fixture = Fixture::new();
        fs::write(
            fixture.root.join("library.json"),
            br#"{"packs":[{"id":"../../outside","name":"Escape"}],"selected":"../../outside"}"#,
        )
        .unwrap();

        let state = fixture.library().state().unwrap();
        assert!(state.packs.is_empty());
        assert!(state.selected.is_none());
    }
}
