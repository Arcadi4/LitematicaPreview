use std::path::Path;

use litematica_preview::preview::{decode, PackCache, MAX_ATLAS_SIDE};
use litematica_preview_core::{MeshBatch, PreviewOptions};

#[test]
fn texture_atlas_grows_within_device_limit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pack = root.join("../Assets/pack.zip");
    let schematic = root.join("../Fixtures/Formats/Sponge.schem");
    let mut packs = PackCache::default();
    let load =
        |packs: &mut PackCache, limit, consume: &mut dyn FnMut(MeshBatch) -> Result<(), String>| {
            decode(
                &schematic,
                &pack,
                &[],
                packs,
                PreviewOptions::default(),
                limit,
                || Ok(()),
                |_, _| Ok(()),
                consume,
                |_, _, _| {},
            )
        };
    let error = load(&mut packs, 32, &mut |_| Ok(())).unwrap_err();
    assert!(error.contains("texture atlas limit"), "{error}");

    let mut atlas_side = 0;
    let result = load(&mut packs, MAX_ATLAS_SIDE, &mut |batch: MeshBatch| {
        if let Some(atlas) = batch.textures.iter().find(|texture| texture.index == 0) {
            atlas_side = atlas.width;
        }
        Ok(())
    });
    assert!(result.is_ok());
    assert!(atlas_side > 32 && atlas_side <= MAX_ATLAS_SIDE);
}
