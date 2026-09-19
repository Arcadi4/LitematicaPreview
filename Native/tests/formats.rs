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

#[test]
fn abi_layout_matches_x64_csharp_and_errors_are_owned() {
    assert_eq!(std::mem::size_of::<PreviewInfo>(), 56);
    assert_eq!(std::mem::size_of::<PartView>(), 56);
    assert_eq!(std::mem::size_of::<TextureView>(), 24);
    let mut pack = std::ptr::null_mut();
    let mut error = ErrorBuffer::default();
    let invalid = b"not a resource pack";
    unsafe {
        assert_ne!(
            lp_pack_open(invalid.as_ptr(), invalid.len(), &mut pack, &mut error),
            0
        );
        assert!(pack.is_null());
        assert!(!error.data.is_null());
        lp_error_free(error.data, error.len);
        lp_preview_free(std::ptr::null_mut());
        lp_pack_free(std::ptr::null_mut());
    }
}
