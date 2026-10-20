use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::preview::PackLayer;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexturePack {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexturePackState {
    /// Highest priority first. Enabled packs override the bundled resources.
    pub packs: Vec<TexturePack>,
}

/// A directory of validated resource-pack ZIPs and their saved stack.
/// Every mutation loads and saves the index under one lock.
#[derive(Clone)]
pub struct TexturePackLibrary {
    directory: PathBuf,
    lock: Arc<Mutex<()>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedState {
    #[serde(default)]
    packs: Vec<PersistedPack>,
    #[serde(default)]
    selected: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedPack {
    id: String,
    name: String,
    #[serde(default)]
    enabled: Option<bool>,
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

    /// Resolves the enabled archives highest priority first. A missing saved
    /// copy is an error so the preview cannot silently use other resources.
    pub fn layers(&self) -> Result<Vec<PackLayer>, String> {
        let _guard = self.lock.lock();
        let state = self.load()?;
        self.layers_for(&state)
    }

    /// Stores an archive after `validate` accepts its bytes. The pack joins the
    /// stack at the lowest priority and enabled, so it overrides vanilla only.
    /// A rejected or unsaved archive leaves no entry behind.
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
        let pack = TexturePack {
            id,
            name,
            enabled: true,
        };
        state.packs.push(pack.clone());
        if let Err(error) = self.save(&state) {
            let _ = fs::remove_file(&archive);
            return Err(error);
        }
        Ok(pack)
    }

    /// Validates the prospective enabled stack without the index lock.
    /// A concurrent library change rejects the save and preserves that change.
    pub fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        validate: impl FnOnce(&[PackLayer]) -> Result<(), String>,
    ) -> Result<TexturePackState, String> {
        let snapshot = {
            let _guard = self.lock.lock();
            self.load()?
        };
        let position = snapshot
            .packs
            .iter()
            .position(|pack| pack.id == id)
            .ok_or("The texture pack is not in the library.")?;
        if snapshot.packs[position].enabled == enabled {
            return Ok(snapshot);
        }
        let mut next = snapshot.clone();
        next.packs[position].enabled = enabled;
        if enabled {
            validate(&self.layers_for(&next)?)?;
        }
        let _guard = self.lock.lock();
        if self.load()? != snapshot {
            return Err("The texture pack library changed. Try the change again.".into());
        }
        self.save(&next)?;
        Ok(next)
    }

    /// Replaces the priority order with exactly the saved pack IDs.
    pub fn set_order(&self, ids: &[String]) -> Result<TexturePackState, String> {
        let _guard = self.lock.lock();
        let mut state = self.load()?;
        if ids.len() != state.packs.len() {
            return Err("The texture pack order no longer matches the library.".into());
        }
        let mut remaining: Vec<Option<TexturePack>> = state.packs.drain(..).map(Some).collect();
        let mut ordered = Vec::with_capacity(ids.len());
        for id in ids {
            let position = remaining
                .iter()
                .position(|pack| pack.as_ref().is_some_and(|pack| &pack.id == id))
                .ok_or("The order contains an unknown or duplicate texture pack.")?;
            ordered.push(
                remaining[position]
                    .take()
                    .ok_or("The order contains an unknown or duplicate texture pack.")?,
            );
        }
        state.packs = ordered;
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
        self.save(&state)?;
        let _ = fs::remove_file(self.archive_path(id));
        Ok(state)
    }

    fn layers_for(&self, state: &TexturePackState) -> Result<Vec<PackLayer>, String> {
        state
            .packs
            .iter()
            .filter(|pack| pack.enabled)
            .map(|pack| {
                let path = self.archive_path(&pack.id);
                if !path.is_file() {
                    return Err(format!(
                        "The saved copy of “{}” is missing. Import the texture pack again or disable it.",
                        pack.name
                    ));
                }
                Ok(PackLayer {
                    name: pack.name.clone(),
                    path,
                })
            })
            .collect()
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
        let Ok(persisted) = serde_json::from_slice::<PersistedState>(&bytes) else {
            let _ = fs::rename(&index, self.directory.join("library.invalid.json"));
            return Ok(TexturePackState::default());
        };
        // A legacy index has no enabled flags and one selected pack. Enable
        // exactly that pack. A null or unknown selection enables nothing.
        let legacy = persisted.packs.iter().any(|pack| pack.enabled.is_none());
        let selected = persisted.selected.clone();
        let selected = selected.as_deref();
        let mut seen = HashSet::new();
        let packs = persisted
            .packs
            .into_iter()
            .filter_map(|pack| {
                if Uuid::parse_str(&pack.id).is_err() || !seen.insert(pack.id.clone()) {
                    return None;
                }
                let enabled = if legacy {
                    selected == Some(pack.id.as_str())
                } else {
                    pack.enabled.unwrap_or(false)
                };
                Some(TexturePack {
                    id: pack.id,
                    name: pack.name,
                    enabled,
                })
            })
            .collect();
        Ok(TexturePackState { packs })
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
    use nucleation::meshing::ResourcePackSource;
    use std::path::Path;

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

        fn write_index(&self, contents: &str) {
            fs::write(self.root.join("library.json"), contents).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn imports_join_the_stack_enabled_at_lowest_priority() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let first = library.import("Cottage", b"first", |_| Ok(())).unwrap();
        let second = library.import("Cottage", b"second", |_| Ok(())).unwrap();
        assert_eq!(first.name, "Cottage");
        assert_eq!(second.name, "Cottage (2)");
        assert!(first.enabled && second.enabled);
        assert!(fixture.root.join(format!("{}.zip", first.id)).is_file());
        assert!(fixture.root.join(format!("{}.zip", second.id)).is_file());

        let state = library.state().unwrap();
        assert_eq!(state.packs, [first.clone(), second.clone()]);
        let layers = library.layers().unwrap();
        assert_eq!(
            layers
                .iter()
                .map(|layer| layer.name.as_str())
                .collect::<Vec<_>>(),
            ["Cottage", "Cottage (2)"]
        );

        let restored = fixture.library().state().unwrap();
        assert_eq!(restored, state);
        assert_eq!(restored.packs, [first, second]);
    }

    #[test]
    fn accepts_and_reloads_a_real_resource_pack() {
        use litematica_preview_core::resource_pack_overlay;

        let fixture = Fixture::new();
        let library = fixture.library();
        let bytes = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../Assets/pack.zip"))
            .expect("bundled resource pack");
        let empty = ResourcePackSource::from_resource_pack(Default::default());
        let pack = library
            .import("Vanilla copy", &bytes, |bytes| {
                resource_pack_overlay(&empty, bytes).map(|_| ())
            })
            .expect("import bundled resource pack");

        let state = library.state().unwrap();
        assert_eq!(state.packs, [pack.clone()]);
        assert!(state.packs[0].enabled);
        assert_eq!(
            library.layers().expect("enabled stack"),
            [PackLayer {
                name: "Vanilla copy".into(),
                path: fixture.root.join(format!("{}.zip", pack.id)),
            }]
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
        assert!(library.layers().unwrap().is_empty());
        assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
    }

    #[test]
    fn failed_enable_validation_keeps_the_previous_state() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let pack = library.import("Template", b"data", |_| Ok(())).unwrap();
        library.set_enabled(&pack.id, false, |_| Ok(())).unwrap();

        let mut seen = Vec::new();
        let error = library
            .set_enabled(&pack.id, true, |layers| {
                seen = layers.iter().map(|layer| layer.name.clone()).collect();
                Err("The pack has no usable assets.".into())
            })
            .unwrap_err();
        assert_eq!(error, "The pack has no usable assets.");
        assert_eq!(seen, vec!["Template".to_owned()]);
        assert!(!library.state().unwrap().packs[0].enabled);
        assert!(library.set_enabled("missing", true, |_| Ok(())).is_err());

        let enabled = library
            .set_enabled(&pack.id, true, |layers| {
                assert_eq!(layers.len(), 1);
                Ok(())
            })
            .unwrap();
        assert!(enabled.packs[0].enabled);
        assert_eq!(library.layers().unwrap().len(), 1);
    }

    #[test]
    fn enabling_does_not_overwrite_changes_during_validation() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let original = library.import("Original", b"original", |_| Ok(())).unwrap();
        library
            .set_enabled(&original.id, false, |_| Ok(()))
            .unwrap();

        let result = library.set_enabled(&original.id, true, |_| {
            library.import("Added", b"added", |_| Ok(()))?;
            Ok(())
        });

        assert!(result.is_err());
        let restored = fixture.library().state().unwrap();
        assert_eq!(restored.packs.len(), 2);
        assert_eq!(restored.packs[0].id, original.id);
        assert!(!restored.packs[0].enabled);
        assert_eq!(restored.packs[1].name, "Added");
        assert!(restored.packs[1].enabled);
    }

    #[test]
    fn order_rejects_unknown_and_duplicate_ids_without_saving() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let first = library.import("First", b"one", |_| Ok(())).unwrap();
        let second = library.import("Second", b"two", |_| Ok(())).unwrap();
        let original = library.state().unwrap();

        assert!(library
            .set_order(&[first.id.clone(), first.id.clone()])
            .is_err());
        assert!(library
            .set_order(&[first.id.clone(), "missing".into()])
            .is_err());
        assert_eq!(library.state().unwrap(), original);

        let ordered = library
            .set_order(&[second.id.clone(), first.id.clone()])
            .unwrap();
        assert_eq!(
            ordered
                .packs
                .iter()
                .map(|pack| pack.id.as_str())
                .collect::<Vec<_>>(),
            [second.id.as_str(), first.id.as_str()]
        );
        assert_eq!(fixture.library().state().unwrap(), ordered);
    }

    #[test]
    fn removing_a_pack_deletes_its_archive_and_keeps_the_rest() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let first = library.import("First", b"one", |_| Ok(())).unwrap();
        let second = library.import("Second", b"two", |_| Ok(())).unwrap();

        let state = library.remove(&first.id).unwrap();
        assert_eq!(state.packs, [second.clone()]);
        assert!(!fixture.root.join(format!("{}.zip", first.id)).exists());
        assert!(fixture.root.join(format!("{}.zip", second.id)).is_file());
    }

    #[test]
    fn migrates_the_legacy_selected_pack_to_an_enabled_stack() {
        let fixture = Fixture::new();
        let selected = Uuid::new_v4().to_string();
        let other = Uuid::new_v4().to_string();
        fs::write(fixture.root.join(format!("{selected}.zip")), b"selected").unwrap();
        fs::write(fixture.root.join(format!("{other}.zip")), b"other").unwrap();
        fixture.write_index(&format!(
            r#"{{"packs":[{{"id":"{other}","name":"Other"}},{{"id":"{selected}","name":"Selected"}}],"selected":"{selected}"}}"#
        ));

        let state = fixture.library().state().unwrap();
        assert_eq!(
            state
                .packs
                .iter()
                .map(|pack| (pack.name.as_str(), pack.enabled))
                .collect::<Vec<_>>(),
            [("Other", false), ("Selected", true)]
        );
        assert_eq!(state.packs[0].id, other);
        assert_eq!(state.packs[1].id, selected);

        let library = fixture.library();
        let ordered = library
            .set_order(&[selected.clone(), other.clone()])
            .unwrap();
        assert_eq!(
            ordered
                .packs
                .iter()
                .map(|pack| (pack.id.as_str(), pack.enabled))
                .collect::<Vec<_>>(),
            [(selected.as_str(), true), (other.as_str(), false)]
        );
        assert_eq!(fixture.library().state().unwrap(), ordered);
        assert_eq!(
            library.layers().unwrap(),
            [PackLayer {
                name: "Selected".into(),
                path: fixture.root.join(format!("{selected}.zip")),
            }]
        );
    }

    #[test]
    fn legacy_null_or_unknown_selection_enables_nothing() {
        let fixture = Fixture::new();
        let pack = Uuid::new_v4().to_string();
        fixture.write_index(&format!(
            r#"{{"packs":[{{"id":"{pack}","name":"Saved"}}],"selected":null}}"#
        ));
        let state = fixture.library().state().unwrap();
        assert_eq!(state.packs.len(), 1);
        assert!(!state.packs[0].enabled);
        assert!(fixture.library().layers().unwrap().is_empty());

        fixture.write_index(&format!(
            r#"{{"packs":[{{"id":"{pack}","name":"Saved"}}],"selected":"{}"}}"#,
            Uuid::new_v4()
        ));
        let state = fixture.library().state().unwrap();
        assert_eq!(state.packs.len(), 1);
        assert!(!state.packs[0].enabled);
        assert!(fixture.library().layers().unwrap().is_empty());
    }

    #[test]
    fn missing_enabled_archive_names_the_pack() {
        let fixture = Fixture::new();
        let library = fixture.library();
        let pack = library.import("Cottage", b"data", |_| Ok(())).unwrap();
        fs::remove_file(fixture.root.join(format!("{}.zip", pack.id))).unwrap();

        let error = library.layers().unwrap_err();
        assert!(error.contains("Cottage"), "{error}");
        assert!(error.contains("missing"), "{error}");
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
        fixture.write_index(r#"{"packs":[{"id":"../../outside","name":"Escape","enabled":true}]}"#);

        assert!(fixture.library().state().unwrap().packs.is_empty());
    }
}
