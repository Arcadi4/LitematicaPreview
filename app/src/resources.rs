use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Embeds a repository asset independently of the source file's location.
macro_rules! asset {
    ($path:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../", $path))
    };
}

/// The vanilla block resource pack embedded in the executable. Previews never
/// read an adjacent pack path, so relocatable and self-updated installs work.
pub const VANILLA_PACK: &[u8] = asset!("assets/pack.zip");

/// Mojang sun and full moon, cropped to their luminous discs from the Minecraft Wiki
/// [sun](https://minecraft.wiki/w/File:EnvSprite_sun.png) and
/// [moon](https://minecraft.wiki/w/File:EnvSprite_full-moon.png) sprites.
pub(crate) const SKY_BODIES: &[u8] = asset!("assets/sky-bodies.png");

pub(crate) const APP_ICON: &[u8] = asset!("assets/icon.png");

const DEMOS: &[(&str, &[u8])] = &[
    (
        "Cottage.litematic",
        asset!("fixtures/demos/Cottage.litematic"),
    ),
    ("Archway.schem", asset!("fixtures/demos/Archway.schem")),
    (
        "Watchtower.schematic",
        asset!("fixtures/demos/Watchtower.schematic"),
    ),
    ("Bloom.nbt", asset!("fixtures/demos/Bloom.nbt")),
    ("Bridge.snbt", asset!("fixtures/demos/Bridge.snbt")),
    (
        "Pyramid.mcstructure",
        asset!("fixtures/demos/Pyramid.mcstructure"),
    ),
    ("Garden.nusn", asset!("fixtures/demos/Garden.nusn")),
];

/// Materializes the embedded demos into `directory` and returns their real
/// paths in deterministic order. A file is rewritten atomically only when it
/// is missing or differs from the embedded bytes; unchanged files are left
/// untouched so reopening the app is cheap.
pub fn demos(directory: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(directory).map_err(|error| {
        format!(
            "Unable to create the demo directory {}: {error}",
            directory.display()
        )
    })?;
    let mut paths = Vec::with_capacity(DEMOS.len());
    for &(name, bytes) in DEMOS {
        let path = directory.join(name);
        write_if_changed(&path, bytes)?;
        paths.push(path);
    }
    Ok(paths)
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match std::fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Unable to read the demo {}: {error}",
                path.display()
            ));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("The demo path {} has no parent directory.", path.display()))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("The demo path {} has an invalid filename.", path.display()))?;
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let temp = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        std::fs::write(&temp, bytes)
            .map_err(|error| format!("Unable to write the demo {}: {error}", temp.display()))?;
        std::fs::rename(&temp, path)
            .map_err(|error| format!("Unable to install the demo {}: {error}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
