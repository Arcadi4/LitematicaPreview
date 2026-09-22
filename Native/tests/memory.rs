//! Decoder allocation regression; this executable contains one serial test.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use litematica_preview_native::decode;
use quartz_nbt::{NbtCompound, NbtList, NbtTag};

struct AllocationMeter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn allocated(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for AllocationMeter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                allocated(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: AllocationMeter = AllocationMeter;

fn encoded(root: &NbtCompound) -> Vec<u8> {
    let mut bytes = Vec::new();
    quartz_nbt::io::write_nbt(&mut bytes, None, root, quartz_nbt::io::Flavor::GzCompressed)
        .unwrap();
    bytes
}

fn litematic(volume: usize) -> Vec<u8> {
    let mut size = NbtCompound::new();
    size.insert("x", 256i32);
    size.insert("y", 32i32);
    size.insert("z", 256i32);
    let mut position = NbtCompound::new();
    for axis in ["x", "y", "z"] {
        position.insert(axis, 0i32);
    }
    let mut stone = NbtCompound::new();
    stone.insert("Name", "minecraft:stone");
    let mut region = NbtCompound::new();
    region.insert("Size", size);
    region.insert("Position", position);
    region.insert(
        "BlockStatePalette",
        NbtList::from(vec![NbtTag::Compound(stone)]),
    );
    region.insert(
        "BlockStates",
        NbtTag::LongArray(vec![0; (volume * 2).div_ceil(64)]),
    );
    let mut regions = NbtCompound::new();
    regions.insert("Main", region);
    let mut root = NbtCompound::new();
    root.insert("Version", 6i32);
    root.insert("Metadata", NbtCompound::new());
    root.insert("Regions", regions);
    encoded(&root)
}

fn sponge(volume: usize) -> Vec<u8> {
    let mut root = NbtCompound::new();
    root.insert("Version", 2i32);
    root.insert("Width", 256i16);
    root.insert("Height", 32i16);
    root.insert("Length", 256i16);
    let mut palette = NbtCompound::new();
    palette.insert("minecraft:stone", 0i32);
    root.insert("Palette", palette);
    root.insert("BlockData", NbtTag::ByteArray(vec![0; volume]));
    encoded(&root)
}

#[test]
fn decoded_volume_does_not_require_a_second_dense_array() {
    let volume = 256 * 32 * 256;
    let dense_bytes = volume * std::mem::size_of::<usize>();
    for (format, bytes) in [("litematic", litematic(volume)), ("sponge", sponge(volume))] {
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        let schematic = decode(&bytes).unwrap();
        let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
        assert_eq!(schematic.total_blocks() as usize, volume);
        // One dense array plus packed NBT and bounded parse scratch fit below
        // 1.5 arrays. The former preallocate-then-replace path necessarily fails.
        assert!(
            peak < dense_bytes + dense_bytes / 2,
            "{format}: {peak} peak allocated bytes for {dense_bytes} dense bytes"
        );
    }
}
