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

#[cfg(test)]
mod tests {
    use super::EXTENSIONS;

    #[test]
    fn includes_minecraft_anvil_regions() {
        assert!(EXTENSIONS.contains(&".mca"));
    }
}
