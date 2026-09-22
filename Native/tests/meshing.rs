use litematica_preview_native::load_chunks;
use std::cell::Cell;

#[path = "../src/meshing/tests/fixtures.rs"]
mod fixtures;
use fixtures::{schematic, test_pack};

#[test]
fn native_stream_counts_visible_bounds_and_stops_between_chunks() {
    let pack = test_pack();
    let mut schematic = schematic(&[
        (-1, 0, 0, "minecraft:stone"),
        (64, 0, 0, "minecraft:stone"),
        (128, 0, 0, "minecraft:cave_air"),
        (129, 0, 0, "minecraft:void_air"),
    ]);
    schematic.add_block_entity(nucleation::block_entity::BlockEntity::new(
        "minecraft:chest".into(),
        (-1, 0, 0),
    ));
    let data = nucleation::formats::litematic::to_litematic(&schematic).unwrap();
    let mut triangles = 0;
    let mut chunk_count = 0;
    let info = load_chunks(
        &data,
        &pack,
        |preview| {
            chunk_count += 1;
            triangles += preview.info.triangle_count;
            Ok(())
        },
        || Ok(()),
    )
    .unwrap();
    assert_eq!(chunk_count, 2);
    // Decode counts retain their upstream meaning; cave/void air do not emit
    // geometry, affect visible bounds, or create an extra streamed chunk.
    assert_eq!(info.block_count, 4);
    assert_eq!(info.block_entity_count, 1);
    assert_eq!(info.triangle_count, triangles);
    assert_eq!(info.min, [-1.5, -0.5, -0.5]);
    assert_eq!(info.max, [64.5, 0.5, 0.5]);

    let consumed = Cell::new(0);
    let result = load_chunks(
        &data,
        &pack,
        |_| {
            consumed.set(consumed.get() + 1);
            Ok(())
        },
        || {
            if consumed.get() > 0 {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(result.err().as_deref(), Some("cancelled"));
    assert_eq!(consumed.get(), 1);
}

#[test]
fn consumer_failure_stops_streaming_without_accepting_another_chunk() {
    let pack = test_pack();
    let schematic = schematic(&[(0, 0, 0, "minecraft:stone"), (64, 0, 0, "minecraft:stone")]);
    let data = nucleation::formats::litematic::to_litematic(&schematic).unwrap();
    let mut consumed = 0;
    let result = load_chunks(
        &data,
        &pack,
        |_| {
            consumed += 1;
            Err("transport closed".into())
        },
        || Ok(()),
    );
    assert_eq!(result.err().as_deref(), Some("transport closed"));
    assert_eq!(consumed, 1);
}
