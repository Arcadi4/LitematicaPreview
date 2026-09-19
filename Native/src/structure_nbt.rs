// Vanilla Java structure `.nbt` decoding.
//
// Nucleation's `FormatManager` identifies six of the seven formats by
// content; for vanilla structures it reads only the SNBT spelling, so the
// binary container is decoded here into the same data model. Ported from
// schematic-diff's fallback loader; the one deviation is a bounded gunzip in
// the format sniff, where the original probes unbounded.

use std::borrow::Cow;
use std::io::{Cursor, Read};

use flate2::read::GzDecoder;
use nucleation::block_entity::BlockEntity;
use nucleation::block_position::BlockPosition;
use nucleation::nbt::NbtMap;
use nucleation::{BlockState, Entity, Region, UniversalSchematic};
use quartz_nbt::io::Flavor;
use quartz_nbt::{NbtCompound, NbtList, NbtTag};

use super::{DecodeFailure, MAX_DECOMPRESSED_BYTES, MAX_VOLUME};

// None means the input is not a binary Java structure.
pub(super) fn try_load(bytes: &[u8]) -> Option<Result<UniversalSchematic, DecodeFailure>> {
    let raw = maybe_gunzip(bytes).ok()?;
    // Validate nesting and collection sizes before quartz_nbt allocates them.
    validate_nbt(&raw).ok()?;
    let (root, _) = quartz_nbt::io::read_nbt(&mut Cursor::new(&raw), Flavor::Uncompressed).ok()?;
    if !root.contains_key("blocks") || !root.contains_key("palette") {
        return None;
    }
    Some(load_structure_nbt(root))
}

fn load_structure_nbt(root: NbtCompound) -> Result<UniversalSchematic, DecodeFailure> {
    let unreadable =
        || DecodeFailure::Format("This file is not a Java structure block container.".to_string());

    let size = triple(&root, "size").ok_or_else(unreadable)?;
    if size.iter().any(|axis| *axis <= 0) {
        return Err(unreadable());
    }
    let volume = super::preview_limits()
        .check_dimensions((i64::from(size[0]), i64::from(size[1]), i64::from(size[2])))
        .map_err(|e| DecodeFailure::Limit(e.to_string()))? as i64;
    if volume > i64::try_from(MAX_VOLUME).expect("volume fits") {
        return Err(DecodeFailure::Limit(format!(
            "This schematic is {size:?} ({volume} cells), beyond the {MAX_VOLUME}-cell preview limit."
        )));
    }

    let data_version = match root.inner().get("DataVersion") {
        Some(NbtTag::Int(value)) => Some(*value),
        _ => None,
    };

    let mut schematic = UniversalSchematic::new("structure".to_string());
    schematic.metadata.name = Some("structure".to_string());
    schematic.metadata.mc_version = data_version;
    schematic.metadata.source_data_version = data_version;
    schematic.default_region = Region::try_new(
        schematic.default_region_name.clone(),
        (0, 0, 0),
        (size[0], size[1], size[2]),
    )
    .map_err(|error| DecodeFailure::Format(format!("{error}")))?;

    let palette = read_palette(&root).ok_or_else(unreadable)?;

    let Some(NbtTag::List(blocks)) = root.inner().get("blocks") else {
        return Err(unreadable());
    };
    if blocks.len() as i64 > volume {
        return Err(unreadable());
    }

    for entry in blocks.iter() {
        let NbtTag::Compound(entry) = entry else {
            return Err(unreadable());
        };
        let Some(position) = triple(entry, "pos") else {
            return Err(unreadable());
        };
        // Positions are validated against `size` in shape above only; a
        // malformed file can still point outside the grid.
        if position
            .iter()
            .enumerate()
            .any(|(axis, value)| *value < 0 || *value >= size[axis])
        {
            return Err(DecodeFailure::Format(format!(
                "This structure has a block at {position:?}, outside its {size:?} size."
            )));
        }
        let Some(NbtTag::Int(index)) = entry.inner().get("state") else {
            return Err(unreadable());
        };
        let Some(state) = palette.get(*index as usize) else {
            return Err(unreadable());
        };

        let block = BlockState::from_block_string(state)
            .map_err(|error| DecodeFailure::Format(format!("{error}")))?;
        let (x, y, z) = (position[0], position[1], position[2]);
        schematic.set_block(x, y, z, &block);

        if let Some(NbtTag::Compound(nbt)) = entry.inner().get("nbt") {
            let map = NbtMap::from_quartz_nbt(nbt);
            let id = nbt
                .inner()
                .get("id")
                .or_else(|| nbt.inner().get("Id"))
                .and_then(|tag| match tag {
                    NbtTag::String(value) => Some(value.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| block.get_name().to_string());
            let mut block_entity = BlockEntity::new(id, (x, y, z));
            block_entity.set_nbt(map);
            schematic.set_block_entity(BlockPosition { x, y, z }, block_entity);
        }
    }

    if let Some(NbtTag::List(list)) = root.inner().get("entities") {
        for entry in list.iter() {
            let NbtTag::Compound(entry) = entry else {
                continue;
            };
            let Some(NbtTag::Compound(nbt)) = entry.inner().get("nbt") else {
                continue;
            };
            // Vanilla ignores entity records with no type id; so do we, and we
            // do not let one odd record fail the whole load.
            if !nbt.contains_key("id") && !nbt.contains_key("Id") {
                continue;
            }
            let mut nbt = nbt.clone();
            if let Some(position) = double_triple(entry, "pos") {
                let coordinates = position.map(NbtTag::Double);
                nbt.insert("Pos", NbtList::clone_from(&coordinates));
            }
            if let Ok(entity) = Entity::from_nbt(&nbt) {
                schematic.add_entity(entity);
            }
        }
    }

    super::preview_limits()
        .validate_schematic(&schematic)
        .map_err(|e| DecodeFailure::Limit(e.to_string()))?;
    Ok(schematic)
}

// Gunzip the bytes when they carry the gzip magic, otherwise pass through.
fn maybe_gunzip(bytes: &[u8]) -> Result<Cow<'_, [u8]>, String> {
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut out = Vec::new();
    GzDecoder::new(Cursor::new(bytes))
        .take(MAX_DECOMPRESSED_BYTES as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|error| format!("gzip decompression failed: {error}"))?;
    if out.len() > MAX_DECOMPRESSED_BYTES {
        return Err(format!(
            "the structure expands beyond the {} MiB preview limit.",
            MAX_DECOMPRESSED_BYTES / 1_048_576
        ));
    }
    Ok(Cow::Owned(out))
}

// Read an `[i32; 3]` from an int array or a list of int-ish tags.
fn triple(compound: &NbtCompound, key: &str) -> Option<[i32; 3]> {
    let tag = compound.inner().get(key)?;
    let values: Vec<i32> = match tag {
        NbtTag::IntArray(values) => values.clone(),
        NbtTag::List(list) => {
            let mut out = Vec::with_capacity(list.len());
            for entry in list.iter() {
                out.push(match entry {
                    NbtTag::Byte(v) => i32::from(*v),
                    NbtTag::Short(v) => i32::from(*v),
                    NbtTag::Int(v) => *v,
                    _ => return None,
                });
            }
            out
        }
        _ => return None,
    };
    let [x, y, z] = values.as_slice() else {
        return None;
    };
    Some([*x, *y, *z])
}

// Read an `[f64; 3]` from a list of float-ish tags.
fn double_triple(compound: &NbtCompound, key: &str) -> Option<[f64; 3]> {
    let NbtTag::List(list) = compound.inner().get(key)? else {
        return None;
    };
    let mut out = Vec::with_capacity(list.len());
    for entry in list.iter() {
        out.push(match entry {
            NbtTag::Float(v) => f64::from(*v),
            NbtTag::Double(v) => *v,
            _ => return None,
        });
    }
    let [x, y, z] = out.as_slice() else {
        return None;
    };
    Some([*x, *y, *z])
}

// Read the block-state palette as canonical `id[property=value,…]` strings.
fn read_palette(root: &NbtCompound) -> Option<Vec<String>> {
    let NbtTag::List(palette) = root.inner().get("palette")? else {
        return None;
    };
    let mut states = Vec::with_capacity(palette.len());
    for entry in palette.iter() {
        let NbtTag::Compound(entry) = entry else {
            return None;
        };
        let name = match entry.inner().get("Name") {
            Some(NbtTag::String(name)) => name.clone(),
            _ => return None,
        };
        let mut properties: Vec<(String, String)> = Vec::new();
        if let Some(NbtTag::Compound(properties_tag)) = entry.inner().get("Properties") {
            for (key, value) in properties_tag.inner().iter() {
                let NbtTag::String(value) = value else {
                    return None;
                };
                properties.push((key.clone(), value.clone()));
            }
        }
        // Property order is not meaningful, and sorting it means two files
        // that differ only in serialization order compare equal.
        properties.sort();
        states.push(if properties.is_empty() {
            name
        } else {
            let body = properties
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(",");
            format!("{name}[{body}]")
        });
    }
    Some(states)
}

// A no-allocation structural pass prevents a malformed length/depth from
// reaching quartz_nbt's unbounded reader. NBT tag numbers are explicit.
fn validate_nbt(bytes: &[u8]) -> Result<(), ()> {
    struct Scan<'a> {
        rest: &'a [u8],
        nodes: usize,
    }
    impl Scan<'_> {
        fn take(&mut self, n: usize) -> Result<&[u8], ()> {
            if n > self.rest.len() {
                return Err(());
            }
            let (head, tail) = self.rest.split_at(n);
            self.rest = tail;
            Ok(head)
        }
        fn byte(&mut self) -> Result<u8, ()> {
            Ok(self.take(1)?[0])
        }
        fn string(&mut self) -> Result<(), ()> {
            let n = u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| ())?) as usize;
            self.take(n)?;
            Ok(())
        }
        fn count(&mut self) -> Result<usize, ()> {
            let n = i32::from_be_bytes(self.take(4)?.try_into().map_err(|_| ())?);
            if n < 0 || n as usize > super::MAX_NBT_COLLECTION_ITEMS {
                return Err(());
            }
            Ok(n as usize)
        }
        fn payload(&mut self, tag: u8, depth: usize) -> Result<(), ()> {
            self.nodes += 1;
            if depth > super::MAX_NBT_DEPTH || self.nodes > super::MAX_NBT_NODES {
                return Err(());
            }
            match tag {
                1 => {
                    self.take(1)?;
                }
                2 => {
                    self.take(2)?;
                }
                3 | 5 => {
                    self.take(4)?;
                }
                4 | 6 => {
                    self.take(8)?;
                }
                7 | 11 | 12 => {
                    let n = self.count()?;
                    let width = match tag {
                        7 => 1,
                        11 => 4,
                        _ => 8,
                    };
                    self.take(n.checked_mul(width).ok_or(())?)?;
                }
                8 => self.string()?,
                9 => {
                    let child = self.byte()?;
                    let n = self.count()?;
                    if n > super::MAX_NBT_NODES.saturating_sub(self.nodes) {
                        return Err(());
                    }
                    for _ in 0..n {
                        self.payload(child, depth + 1)?;
                    }
                }
                10 => loop {
                    let child = self.byte()?;
                    if child == 0 {
                        break;
                    }
                    self.string()?;
                    self.payload(child, depth + 1)?;
                },
                _ => return Err(()),
            }
            Ok(())
        }
    }
    let mut scan = Scan {
        rest: bytes,
        nodes: 0,
    };
    if scan.byte()? != 10 {
        return Err(());
    }
    scan.string()?;
    scan.payload(10, 0)
}
