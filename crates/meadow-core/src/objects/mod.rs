//! Custom objects (trees, rocks, cliff pieces, ruins...) loaded from schematics, plus their
//! rotated/mirrored variants. Placement logic lives in `crate::decorate` and is shared by the
//! exporter and the 3D preview.

pub mod legacy;

use std::path::Path;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use meadow_blocks::{BlockState, StateId};
use meadow_nbt::{Compound, Tag};
use serde::{Deserialize, Serialize};

/// A block inside an object, relative to the object's minimum corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjBlock {
    pub x: u16,
    pub y: u16,
    pub z: u16,
    pub state: StateId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CustomObject {
    pub name: String,
    /// Size along x (east), y (up), z (south).
    pub size: [u16; 3],
    /// Non-air blocks.
    pub blocks: Vec<ObjBlock>,
    /// The block (relative to the min corner) that is placed on the column above the surface.
    pub origin: [i32; 3],
    /// Origin stored in the file (WorldEdit offset), if any.
    pub file_origin: Option<[i32; 3]>,
    pub source: Option<String>,
    #[serde(skip)]
    variants: [OnceLock<Arc<Variant>>; 8],
}

impl Clone for CustomObject {
    fn clone(&self) -> Self {
        CustomObject {
            name: self.name.clone(),
            size: self.size,
            blocks: self.blocks.clone(),
            origin: self.origin,
            file_origin: self.file_origin,
            source: self.source.clone(),
            variants: Default::default(),
        }
    }
}

/// A rotated/mirrored copy of an object with block positions relative to its origin.
#[derive(Debug)]
pub struct Variant {
    pub blocks: Vec<(i32, i32, i32, StateId)>,
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl CustomObject {
    pub fn new(name: impl Into<String>, size: [u16; 3], blocks: Vec<ObjBlock>) -> Self {
        let mut o = CustomObject {
            name: name.into(),
            size,
            blocks,
            origin: [0, 0, 0],
            file_origin: None,
            source: None,
            variants: Default::default(),
        };
        o.origin = o.default_origin();
        o
    }

    /// Build from arbitrary positioned blocks (normalises to a min corner of 0).
    pub fn from_positions(name: impl Into<String>, list: &[(i32, i32, i32, StateId)], origin: [i32; 3]) -> Self {
        let min = list.iter().fold([i32::MAX; 3], |m, b| [m[0].min(b.0), m[1].min(b.1), m[2].min(b.2)]);
        let max = list.iter().fold([i32::MIN; 3], |m, b| [m[0].max(b.0), m[1].max(b.1), m[2].max(b.2)]);
        let min = if list.is_empty() { [0; 3] } else { min };
        let blocks = list
            .iter()
            .filter(|b| !b.3.is_air())
            .map(|b| ObjBlock { x: (b.0 - min[0]) as u16, y: (b.1 - min[1]) as u16, z: (b.2 - min[2]) as u16, state: b.3 })
            .collect();
        let size = if list.is_empty() {
            [1, 1, 1]
        } else {
            [(max[0] - min[0] + 1) as u16, (max[1] - min[1] + 1) as u16, (max[2] - min[2] + 1) as u16]
        };
        let mut o = CustomObject::new(name, size, blocks);
        o.origin = [origin[0] - min[0], origin[1] - min[1], origin[2] - min[2]];
        o
    }

    /// Horizontal centre of the lowest layer that contains blocks.
    pub fn default_origin(&self) -> [i32; 3] {
        let lowest = self.blocks.iter().map(|b| b.y).min().unwrap_or(0) as i32;
        [self.size[0] as i32 / 2, lowest, self.size[2] as i32 / 2]
    }

    /// Variant index: rotation (0..4, clockwise quarter turns) + 4 * mirror.
    pub fn variant(&self, rotation: u8, mirror: bool) -> Arc<Variant> {
        let idx = (rotation & 3) as usize + if mirror { 4 } else { 0 };
        self.variants[idx].get_or_init(|| Arc::new(self.build_variant(rotation & 3, mirror))).clone()
    }

    fn build_variant(&self, rot: u8, mirror: bool) -> Variant {
        let mut cache: rustc_hash::FxHashMap<StateId, StateId> = Default::default();
        let mut blocks = Vec::with_capacity(self.blocks.len());
        let mut min = [i32::MAX; 3];
        let mut max = [i32::MIN; 3];
        for b in &self.blocks {
            let mut x = b.x as i32 - self.origin[0];
            let y = b.y as i32 - self.origin[1];
            let mut z = b.z as i32 - self.origin[2];
            if mirror {
                x = -x;
            }
            for _ in 0..rot {
                (x, z) = (-z, x);
            }
            let s = *cache.entry(b.state).or_insert_with(|| transform_state(b.state, rot, mirror));
            for (i, v) in [x, y, z].into_iter().enumerate() {
                min[i] = min[i].min(v);
                max[i] = max[i].max(v);
            }
            blocks.push((x, y, z, s));
        }
        if blocks.is_empty() {
            min = [0; 3];
            max = [0; 3];
        }
        Variant { blocks, min, max }
    }

    /// Largest horizontal distance from the origin over all rotations.
    pub fn radius(&self) -> i32 {
        let v = self.variant(0, false);
        v.min[0].abs().max(v.max[0].abs()).max(v.min[2].abs()).max(v.max[2].abs()) + 1
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn load(path: &Path) -> Result<CustomObject> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "object".into());
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut obj = match ext.as_str() {
            "bo2" => parse_bo2(&name, &String::from_utf8_lossy(&data))?,
            _ => {
                let (_, root) = meadow_nbt::from_any(&data).context("not an NBT schematic")?;
                parse_nbt_object(&name, &root)?
            }
        };
        obj.source = Some(path.display().to_string());
        Ok(obj)
    }
}

/// Detect and parse any supported NBT object format.
pub fn parse_nbt_object(name: &str, root: &Compound) -> Result<CustomObject> {
    // Sponge v3 wraps everything in a "Schematic" compound.
    let r = root.compound("Schematic").unwrap_or(root);
    if r.contains("Blocks") && r.compound("Blocks").is_some() {
        return parse_sponge(name, r, true);
    }
    if r.contains("Palette") && r.contains("BlockData") {
        return parse_sponge(name, r, false);
    }
    if root.contains("size") && (root.contains("palette") || root.contains("palettes")) {
        return parse_structure(name, root);
    }
    if root.contains("Blocks") && root.contains("Width") {
        return parse_mcedit(name, root);
    }
    bail!("unrecognised schematic format")
}

fn read_varints(data: &[i8], n: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < data.len() && out.len() < n {
        let mut value: u32 = 0;
        let mut shift = 0;
        loop {
            let b = data[i] as u8;
            i += 1;
            value |= ((b & 0x7f) as u32) << shift;
            if b & 0x80 == 0 || i >= data.len() {
                break;
            }
            shift += 7;
        }
        out.push(value);
    }
    out
}

fn parse_sponge(name: &str, r: &Compound, v3: bool) -> Result<CustomObject> {
    let w = r.i32("Width").context("Width")? as u16 as usize;
    let h = r.i32("Height").context("Height")? as u16 as usize;
    let l = r.i32("Length").context("Length")? as u16 as usize;
    let (palette, data) = if v3 {
        let b = r.compound("Blocks").unwrap();
        (b.compound("Palette").context("Palette")?, b.byte_array("Data").context("Data")?)
    } else {
        (r.compound("Palette").context("Palette")?, r.byte_array("BlockData").context("BlockData")?)
    };
    let mut pal: Vec<StateId> = vec![StateId::AIR; palette.len().max(1)];
    for (k, v) in palette.iter() {
        let idx = v.as_i64().unwrap_or(0) as usize;
        if idx >= pal.len() {
            pal.resize(idx + 1, StateId::AIR);
        }
        pal[idx] = StateId::parse(k);
    }
    let ids = read_varints(data, w * h * l);
    let mut blocks = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let s = pal.get(*id as usize).copied().unwrap_or(StateId::AIR);
        if s.is_air() || s.state().path() == "structure_void" {
            continue;
        }
        let x = i % w;
        let z = (i / w) % l;
        let y = i / (w * l);
        blocks.push(ObjBlock { x: x as u16, y: y as u16, z: z as u16, state: s });
    }
    let mut o = CustomObject::new(name, [w as u16, h as u16, l as u16], blocks);
    if let Some(off) = r.int_array("Offset")
        && off.len() == 3
    {
        o.file_origin = Some([-off[0], -off[1], -off[2]]);
    }
    Ok(o)
}

fn parse_structure(name: &str, root: &Compound) -> Result<CustomObject> {
    let size: Vec<i32> = root.list("size").context("size")?.iter().filter_map(|t| t.as_i64().map(|v| v as i32)).collect();
    if size.len() != 3 {
        bail!("bad structure size");
    }
    let palette = match root.list("palette") {
        Some(p) => p,
        None => root.list("palettes").and_then(|p| p.items.first()).and_then(Tag::as_list).context("palette")?,
    };
    let pal: Vec<StateId> = palette
        .iter()
        .map(|t| {
            let c = t.as_compound();
            let name = c.and_then(|c| c.string("Name")).unwrap_or("minecraft:air");
            let props: Vec<(String, String)> = c
                .and_then(|c| c.compound("Properties"))
                .map(|p| p.iter().map(|(k, v)| (k.to_owned(), v.as_str().unwrap_or("").to_owned())).collect())
                .unwrap_or_default();
            let mut bs = BlockState::new(name, &[]);
            bs.props = props;
            bs.props.sort();
            StateId::of(&bs)
        })
        .collect();
    let mut blocks = Vec::new();
    for b in root.list("blocks").context("blocks")?.iter() {
        let Some(c) = b.as_compound() else { continue };
        let Some(pos) = c.list("pos") else { continue };
        let p: Vec<i32> = pos.iter().filter_map(|t| t.as_i64().map(|v| v as i32)).collect();
        let s = pal.get(c.i32("state").unwrap_or(0) as usize).copied().unwrap_or(StateId::AIR);
        if p.len() == 3 && !s.is_air() && s.state().path() != "structure_void" {
            blocks.push(ObjBlock { x: p[0] as u16, y: p[1] as u16, z: p[2] as u16, state: s });
        }
    }
    Ok(CustomObject::new(name, [size[0] as u16, size[1] as u16, size[2] as u16], blocks))
}

fn parse_mcedit(name: &str, root: &Compound) -> Result<CustomObject> {
    let w = root.i32("Width").context("Width")? as u16 as usize;
    let h = root.i32("Height").context("Height")? as u16 as usize;
    let l = root.i32("Length").context("Length")? as u16 as usize;
    let ids = root.byte_array("Blocks").context("Blocks")?;
    let data = root.byte_array("Data").unwrap_or(&[]);
    let add = root.byte_array("AddBlocks");
    // Schematica stores a name mapping for non-vanilla/modded ids.
    let mapping: Option<Vec<(u16, String)>> = root
        .compound("SchematicaMapping")
        .map(|m| m.iter().filter_map(|(k, v)| Some((v.as_i64()? as u16, k.to_owned()))).collect());
    let mut blocks = Vec::new();
    for i in 0..(w * h * l).min(ids.len()) {
        let mut id = ids[i] as u8 as u16;
        if let Some(a) = add {
            let nib = a.get(i >> 1).copied().unwrap_or(0) as u8;
            let hi = if i & 1 == 0 { nib >> 4 } else { nib & 15 };
            id |= (hi as u16) << 8;
        }
        if id == 0 {
            continue;
        }
        let d = data.get(i).copied().unwrap_or(0) as u8;
        let state = match mapping.as_ref().and_then(|m| m.iter().find(|(i2, _)| *i2 == id)) {
            Some((_, n)) if !n.starts_with("minecraft:") => StateId::parse(n),
            _ => legacy::legacy_state(id, d),
        };
        if state.is_air() {
            continue;
        }
        let x = i % w;
        let z = (i / w) % l;
        let y = i / (w * l);
        blocks.push(ObjBlock { x: x as u16, y: y as u16, z: z as u16, state });
    }
    let mut o = CustomObject::new(name, [w as u16, h as u16, l as u16], blocks);
    if let (Some(ox), Some(oy), Some(oz)) = (root.i32("WEOffsetX"), root.i32("WEOffsetY"), root.i32("WEOffsetZ")) {
        o.file_origin = Some([-ox, -oy, -oz]);
    }
    Ok(o)
}

/// BO2: `[DATA]` lines of `x,y,z:id.data` where z is vertical and (0,0,0) is the origin.
pub fn parse_bo2(name: &str, text: &str) -> Result<CustomObject> {
    let mut in_data = false;
    let mut list = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case("[DATA]") {
            in_data = true;
            continue;
        }
        if line.starts_with('[') {
            in_data = false;
            continue;
        }
        if !in_data || line.is_empty() {
            continue;
        }
        let Some((pos, blk)) = line.split_once(':') else { continue };
        let p: Vec<i32> = pos.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if p.len() != 3 {
            continue;
        }
        let (id, data) = match blk.split_once('.') {
            Some((a, b)) => (a.trim().parse::<u16>().unwrap_or(0), b.trim().split('#').next().unwrap_or("0").parse::<u8>().unwrap_or(0)),
            None => (blk.trim().split('#').next().unwrap_or("0").parse::<u16>().unwrap_or(0), 0),
        };
        let s = legacy::legacy_state(id, data);
        if !s.is_air() {
            // BO2 (x, y, z-up) -> Minecraft (x, y-up, z)
            list.push((p[0], p[2], p[1], s));
        }
    }
    if list.is_empty() {
        bail!("no blocks in BO2 object");
    }
    Ok(CustomObject::from_positions(name, &list, [0, 0, 0]))
}

/// Export an object as a Sponge v2 schematic (gzipped NBT).
pub fn write_sponge(obj: &CustomObject) -> Result<Vec<u8>> {
    let (w, h, l) = (obj.size[0] as usize, obj.size[1] as usize, obj.size[2] as usize);
    let mut palette: Vec<StateId> = vec![StateId::AIR];
    let mut idx = vec![0u32; w * h * l];
    for b in &obj.blocks {
        let p = match palette.iter().position(|s| *s == b.state) {
            Some(p) => p,
            None => {
                palette.push(b.state);
                palette.len() - 1
            }
        };
        idx[b.x as usize + b.z as usize * w + b.y as usize * w * l] = p as u32;
    }
    let mut data = Vec::with_capacity(idx.len());
    for mut v in idx {
        loop {
            let mut byte = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                byte |= 0x80;
            }
            data.push(byte as i8);
            if v == 0 {
                break;
            }
        }
    }
    let mut pal = Compound::new();
    for (i, s) in palette.iter().enumerate() {
        pal.insert(s.state().to_string(), i as i32);
    }
    let root = Compound::new()
        .with("Version", 2i32)
        .with("DataVersion", meadow_blocks::McVersion::LATEST.data_version())
        .with("Width", w as i16)
        .with("Height", h as i16)
        .with("Length", l as i16)
        .with("PaletteMax", palette.len() as i32)
        .with("Palette", pal)
        .with("BlockData", data)
        .with("Offset", vec![-obj.origin[0], -obj.origin[1], -obj.origin[2]]);
    Ok(meadow_nbt::to_gzip("Schematic", &root)?)
}

// ---------------------------------------------------------------------------------------------
// Block state rotation

const HORIZ: [&str; 4] = ["north", "east", "south", "west"];

fn rot_dir(d: &str, rot: u8) -> Option<&'static str> {
    let i = HORIZ.iter().position(|h| *h == d)?;
    Some(HORIZ[(i + rot as usize) & 3])
}

fn mirror_dir(d: &str) -> Option<&'static str> {
    match d {
        "east" => Some("west"),
        "west" => Some("east"),
        "north" => Some("north"),
        "south" => Some("south"),
        _ => None,
    }
}

/// Apply mirror (x flip) then `rot` clockwise quarter turns to a block state.
pub fn transform_state(id: StateId, rot: u8, mirror: bool) -> StateId {
    if rot == 0 && !mirror {
        return id;
    }
    let st = id.state();
    if st.props.is_empty() {
        return id;
    }
    let mut props: Vec<(String, String)> = Vec::with_capacity(st.props.len());
    // Horizontal side properties (fences, walls, vines, glass panes...)
    let side = |name: &str| -> Option<&str> { st.prop(name) };
    let has_sides = HORIZ.iter().any(|h| side(h).is_some());
    for (k, v) in &st.props {
        let mut v = v.clone();
        match k.as_str() {
            "facing" | "horizontal_facing" => {
                let mut d = v.clone();
                if mirror && let Some(m) = mirror_dir(&d) {
                    d = m.to_owned();
                }
                if let Some(r) = rot_dir(&d, rot) {
                    d = r.to_owned();
                }
                v = d;
            }
            "axis" if rot & 1 == 1 => {
                v = match v.as_str() {
                    "x" => "z".into(),
                    "z" => "x".into(),
                    o => o.into(),
                }
            }
            "rotation" => {
                if let Ok(mut r) = v.parse::<i32>() {
                    if mirror {
                        r = (16 - r) % 16;
                    }
                    v = ((r + 4 * rot as i32) % 16).to_string();
                }
            }
            "shape" if mirror => {
                v = v.replace("left", "__l").replace("right", "left").replace("__l", "right");
                // Rails: mirror east/west components
                if v.contains("east") || v.contains("west") {
                    v = v.replace("east", "__e").replace("west", "east").replace("__e", "west");
                }
            }
            "hinge" if mirror => {
                v = if v == "left" { "right".into() } else { "left".into() };
            }
            "type" if mirror && (v == "left" || v == "right") => {
                v = if v == "left" { "right".into() } else { "left".into() };
            }
            _ => {}
        }
        if has_sides && HORIZ.contains(&k.as_str()) {
            continue; // handled below
        }
        props.push((k.clone(), v));
    }
    if has_sides {
        for h in HORIZ {
            if let Some(v) = side(h) {
                let mut d = h;
                if mirror {
                    d = mirror_dir(d).unwrap();
                }
                let d = rot_dir(d, rot).unwrap();
                props.push((d.to_owned(), v.to_owned()));
            }
        }
    }
    props.sort();
    StateId::of(&BlockState { name: st.name.clone(), props })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> StateId {
        StateId::parse(x)
    }

    #[test]
    fn rotation() {
        let st = s("oak_stairs[facing=north,half=bottom,shape=inner_left,waterlogged=false]");
        assert_eq!(transform_state(st, 1, false).state().prop("facing"), Some("east"));
        let m = transform_state(s("oak_stairs[facing=east,half=bottom,shape=inner_left,waterlogged=false]"), 0, true);
        assert_eq!(m.state().prop("facing"), Some("west"));
        assert_eq!(m.state().prop("shape"), Some("inner_right"));
        let log = transform_state(s("oak_log[axis=x]"), 1, false);
        assert_eq!(log.state().prop("axis"), Some("z"));
        let fence = transform_state(s("oak_fence[east=false,north=true,south=false,waterlogged=false,west=false]"), 1, false);
        assert_eq!(fence.state().prop("east"), Some("true"));
        assert_eq!(fence.state().prop("north"), Some("false"));
    }

    #[test]
    fn variants_rotate_positions() {
        let o = CustomObject::from_positions("t", &[(0, 0, 0, s("stone")), (2, 0, 0, s("dirt")), (0, 3, 0, s("oak_log[axis=y]"))], [0, 0, 0]);
        assert_eq!(o.size, [3, 4, 1]);
        let v = o.variant(1, false);
        // (2,0,0) rotated clockwise -> (0,0,2)
        assert!(v.blocks.iter().any(|b| (b.0, b.1, b.2) == (0, 0, 2) && b.3 == s("dirt")));
        assert_eq!(o.variant(1, false).blocks.len(), 3);
    }

    #[test]
    fn sponge_roundtrip_and_bo2() {
        let o = CustomObject::from_positions("t", &[(0, 0, 0, s("stone")), (1, 2, 3, s("oak_planks"))], [0, 0, 0]);
        let bytes = write_sponge(&o).unwrap();
        let (_, root) = meadow_nbt::from_any(&bytes).unwrap();
        let back = parse_nbt_object("t", &root).unwrap();
        assert_eq!(back.size, [2, 3, 4]);
        assert_eq!(back.blocks.len(), 2);
        assert_eq!(back.file_origin, Some([0, 0, 0]));
        let bo2 = "[META]\nversion=2.0\n[DATA]\n0,0,0:17.0\n0,0,1:17.0\n1,0,2:18.0\n";
        let b = parse_bo2("tree", bo2).unwrap();
        assert_eq!(b.size, [2, 3, 1]);
        assert!(b.blocks.iter().any(|x| x.state.state().name == "minecraft:oak_log"));
    }

    #[test]
    fn legacy_mapping() {
        assert_eq!(legacy::legacy_state(17, 2).state().to_string(), "minecraft:birch_log[axis=y]");
        assert_eq!(legacy::legacy_state(35, 14).state().to_string(), "minecraft:red_wool");
        assert_eq!(legacy::legacy_state(53, 6).state().to_string(), "minecraft:oak_stairs[facing=south,half=top]");
        assert!(legacy::legacy_state(0, 0).is_air());
    }
}
