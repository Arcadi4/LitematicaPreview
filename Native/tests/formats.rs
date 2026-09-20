use litematica_preview_native::*;
use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn minimal_formats_preserve_known_blocks_and_structure_entities() {
    for entry in fs::read_dir(root().join("Fixtures/Formats")).unwrap() {
        let path = entry.unwrap().path();
        let schematic = decode(&fs::read(&path).unwrap()).unwrap();
        let structure = path.file_stem().unwrap() == "Structure";
        assert_eq!(
            schematic.total_blocks(),
            if structure { 2 } else { 7 },
            "{}",
            path.display()
        );
        if structure {
            assert_eq!(schematic.get_block_entities_as_list().len(), 1);
        }
    }
}
