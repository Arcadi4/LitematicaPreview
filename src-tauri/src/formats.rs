use std::path::Path;

pub const EXTENSIONS: &[&str] = &[
    ".litematic",
    ".schem",
    ".schematic",
    ".nbt",
    ".snbt",
    ".mcstructure",
    ".mca",
    ".nusn",
];

/// Matches an on-disk path against [`EXTENSIONS`], ignoring ASCII case.
pub fn is_supported_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            EXTENSIONS
                .iter()
                .any(|candidate| candidate[1..].eq_ignore_ascii_case(extension))
        })
}

#[cfg(test)]
mod tests {
    use super::EXTENSIONS;

    #[test]
    fn includes_minecraft_anvil_regions() {
        assert!(EXTENSIONS.contains(&".mca"));
    }
}
