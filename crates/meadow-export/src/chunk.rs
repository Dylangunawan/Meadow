//! In-memory chunk with lazily allocated 16^3 sections and conversion to 1.18+ chunk NBT.

use meadow_blocks::{BlockState, McVersion, StateId, StateInfo};
use meadow_nbt::{Compound, List, Tag};
use rustc_hash::FxHashMap;

pub struct ChunkBuffer {
    pub cx: i32,
    pub cz: i32,
    pub min_section: i32,
    pub sections: Vec<Option<Box<[u32; 4096]>>>,
    /// Biome per column (canonical numeric id), 16x16.
    pub biomes: [u8; 256],
}

#[inline]
fn sidx(x: i32, y: i32, z: i32) -> usize {
    (((y & 15) << 8) | (z << 4) | x) as usize
}

impl ChunkBuffer {
    pub fn new(cx: i32, cz: i32, min_y: i32, max_y: i32) -> Self {
        let min_section = min_y.div_euclid(16);
        let max_section = (max_y - 1).div_euclid(16);
        let n = (max_section - min_section + 1) as usize;
        ChunkBuffer { cx, cz, min_section, sections: (0..n).map(|_| None).collect(), biomes: [1; 256] }
    }

    pub fn min_y(&self) -> i32 {
        self.min_section * 16
    }
    pub fn max_y(&self) -> i32 {
        (self.min_section + self.sections.len() as i32) * 16
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> StateId {
        let s = (y >> 4) - self.min_section;
        if s < 0 || s as usize >= self.sections.len() {
            return StateId::AIR;
        }
        match &self.sections[s as usize] {
            Some(sec) => StateId(sec[sidx(x, y, z)]),
            None => StateId::AIR,
        }
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, z: i32, b: StateId) {
        let s = (y >> 4) - self.min_section;
        if s < 0 || s as usize >= self.sections.len() {
            return;
        }
        let sec = &mut self.sections[s as usize];
        if sec.is_none() {
            if b == StateId::AIR {
                return;
            }
            *sec = Some(Box::new([0u32; 4096]));
        }
        sec.as_mut().unwrap()[sidx(x, y, z)] = b.0;
    }

    /// Highest y (exclusive) of any non-air block in the column, or min_y.
    pub fn top(&self, x: i32, z: i32, pred: impl Fn(StateId) -> bool) -> i32 {
        for s in (0..self.sections.len()).rev() {
            if let Some(sec) = &self.sections[s] {
                for ly in (0..16).rev() {
                    let b = StateId(sec[((ly << 8) | (z << 4) | x) as usize]);
                    if b != StateId::AIR && pred(b) {
                        return (self.min_section + s as i32) * 16 + ly + 1;
                    }
                }
            }
        }
        self.min_y()
    }
}

/// Converts interned states to palette NBT for a target version (cached per worker).
pub struct PaletteCache {
    version: McVersion,
    replace_missing: bool,
    map: FxHashMap<u32, Tag>,
    biome_names: Vec<String>,
    pub missing: FxHashMap<String, u64>,
}

impl PaletteCache {
    pub fn new(version: McVersion, replace_missing: bool) -> Self {
        let biomes = meadow_blocks::biomes();
        let biome_names = (0..=255u8)
            .map(|n| biomes.by_numeric(n).map(|b| b.id.clone()).unwrap_or_else(|| "minecraft:plains".into()))
            .collect();
        PaletteCache { version, replace_missing, map: FxHashMap::default(), biome_names, missing: FxHashMap::default() }
    }

    fn entry(&mut self, id: u32) -> Tag {
        if let Some(t) = self.map.get(&id) {
            return t.clone();
        }
        let st = StateId(id).state();
        let tag = self.build(&st);
        self.map.insert(id, tag.clone());
        tag
    }

    fn build(&mut self, st: &BlockState) -> Tag {
        let name = match meadow_blocks::name_for_version(&st.name, self.version) {
            Some(n) => n.into_owned(),
            None => {
                *self.missing.entry(st.name.clone()).or_default() += 1;
                if self.replace_missing {
                    let fb = meadow_blocks::fallback_block(st.path());
                    let mut c = Compound::new().with("Name", fb);
                    if fb.ends_with("_log") {
                        c.insert("Properties", Compound::new().with("axis", "y"));
                    }
                    if fb.ends_with("_leaves") {
                        c.insert("Properties", Compound::new().with("persistent", "true"));
                    }
                    return Tag::Compound(c);
                }
                st.name.clone()
            }
        };
        let mut c = Compound::new().with("Name", name);
        if !st.props.is_empty() {
            let mut p = Compound::with_capacity(st.props.len());
            for (k, v) in &st.props {
                p.insert(k.clone(), v.as_str());
            }
            c.insert("Properties", p);
        }
        Tag::Compound(c)
    }

    pub fn biome_name(&self, n: u8) -> &str {
        &self.biome_names[n as usize]
    }
}

fn pack(indices: impl Iterator<Item = u32>, count: usize, bits: u32) -> Vec<i64> {
    let per = (64 / bits) as usize;
    let mut out = vec![0i64; count.div_ceil(per)];
    for (i, v) in indices.enumerate() {
        let (li, off) = (i / per, (i % per) as u32 * bits);
        out[li] |= ((v as u64) << off) as i64;
    }
    out
}

fn bits_for(n: usize, min: u32) -> u32 {
    let mut b = 0;
    while (1usize << b) < n {
        b += 1;
    }
    b.max(min)
}

/// Serialise to chunk NBT (1.18+ format).
pub fn to_nbt(c: &ChunkBuffer, pal: &mut PaletteCache, infos: &[StateInfo]) -> Compound {
    let version = pal.version;
    let mut sections = List::new();
    // Section biome palettes are the same for every section of the chunk (column biomes).
    let mut biome_pal: Vec<u8> = Vec::new();
    let mut biome_cells = [0u32; 16];
    for cz in 0..4 {
        for cx in 0..4 {
            let b = c.biomes[((cz * 4 + 1) * 16 + cx * 4 + 1) as usize];
            let i = match biome_pal.iter().position(|x| *x == b) {
                Some(i) => i,
                None => {
                    biome_pal.push(b);
                    biome_pal.len() - 1
                }
            };
            biome_cells[(cz * 4 + cx) as usize] = i as u32;
        }
    }
    let biome_tag = {
        let mut bc = Compound::new();
        bc.insert("palette", List::of(biome_pal.iter().map(|b| Tag::from(pal.biome_name(*b))).collect()));
        if biome_pal.len() > 1 {
            let bits = bits_for(biome_pal.len(), 1);
            // 4x4x4 cells, index = (y*4 + z)*4 + x
            let idx = (0..64).map(|i| biome_cells[i % 16]);
            bc.insert("data", pack(idx, 64, bits));
        }
        bc
    };

    let mut ws = [c.min_y(); 256];
    let mut mb = [c.min_y(); 256];
    let mut mbnl = [c.min_y(); 256];
    let mut of = [c.min_y(); 256];
    for (si, sec) in c.sections.iter().enumerate() {
        let y = c.min_section + si as i32;
        let mut s = Compound::new().with("Y", y as i8);
        match sec {
            None => {
                s.insert("block_states", Compound::new().with("palette", List::of(vec![Tag::Compound(Compound::new().with("Name", "minecraft:air"))])));
            }
            Some(blocks) => {
                let mut palette: Vec<u32> = Vec::with_capacity(16);
                let mut lookup: FxHashMap<u32, u32> = FxHashMap::default();
                let mut indices = vec![0u32; 4096];
                let mut last = (u32::MAX, 0u32);
                for (i, b) in blocks.iter().enumerate() {
                    let pi = if *b == last.0 {
                        last.1
                    } else {
                        let pi = *lookup.entry(*b).or_insert_with(|| {
                            palette.push(*b);
                            (palette.len() - 1) as u32
                        });
                        last = (*b, pi);
                        pi
                    };
                    indices[i] = pi;
                    // Heightmaps (index i = y<<8 | z<<4 | x)
                    if *b != 0 {
                        let col = i & 255;
                        let top = y * 16 + (i >> 8) as i32 + 1;
                        let inf = &infos.get(*b as usize).copied().unwrap_or_else(|| StateId(*b).info());
                        if !inf.air {
                            ws[col] = ws[col].max(top);
                            if inf.solid || inf.liquid {
                                mb[col] = mb[col].max(top);
                                if !inf.leaves {
                                    mbnl[col] = mbnl[col].max(top);
                                }
                            }
                            if inf.solid {
                                of[col] = of[col].max(top);
                            }
                        }
                    }
                }
                let mut bs = Compound::new();
                bs.insert("palette", List::of(palette.iter().map(|p| pal.entry(*p)).collect()));
                if palette.len() > 1 {
                    let bits = bits_for(palette.len(), 4);
                    bs.insert("data", pack(indices.into_iter(), 4096, bits));
                }
                s.insert("block_states", bs);
            }
        }
        s.insert("biomes", biome_tag.clone());
        sections.push(s);
    }

    let hm_bits = bits_for((c.max_y() - c.min_y() + 1) as usize, 1);
    let hm = |v: &[i32; 256]| -> Tag {
        let base = c.min_y();
        Tag::LongArray(pack(v.iter().map(|h| (h - base) as u32), 256, hm_bits))
    };
    let heightmaps = Compound::new()
        .with("MOTION_BLOCKING", hm(&mb))
        .with("MOTION_BLOCKING_NO_LEAVES", hm(&mbnl))
        .with("OCEAN_FLOOR", hm(&of))
        .with("WORLD_SURFACE", hm(&ws));

    let mut root = Compound::with_capacity(16);
    root.insert("DataVersion", version.data_version());
    root.insert("xPos", c.cx);
    root.insert("zPos", c.cz);
    root.insert("yPos", c.min_section);
    root.insert("Status", "minecraft:full");
    root.insert("LastUpdate", 0i64);
    root.insert("InhabitedTime", 0i64);
    root.insert("isLightOn", 0i8);
    root.insert("sections", sections);
    root.insert("block_entities", List::new());
    root.insert("Heightmaps", heightmaps);
    root.insert("fluid_ticks", List::new());
    root.insert("block_ticks", List::new());
    root.insert("structures", Compound::new().with("References", Compound::new()).with("starts", Compound::new()));
    root
}

/// Decode a chunk section's block at local coordinates (verification / map import).
pub fn decode_block(chunk: &Compound, x: i32, y: i32, z: i32) -> Option<String> {
    let sy = y.div_euclid(16);
    let sec = chunk.list("sections")?.iter().filter_map(Tag::as_compound).find(|s| s.i32("Y") == Some(sy))?;
    let bs = sec.compound("block_states")?;
    let palette = bs.list("palette")?;
    let idx = match bs.long_array("data") {
        None => 0,
        Some(data) => {
            let bits = bits_for(palette.len(), 4);
            let per = 64 / bits as usize;
            let i = (((y & 15) << 8) | (z << 4) | x) as usize;
            ((data[i / per] as u64 >> ((i % per) as u32 * bits)) & ((1u64 << bits) - 1)) as usize
        }
    };
    let e = palette.items.get(idx)?.as_compound()?;
    let mut s = e.string("Name")?.to_owned();
    if let Some(p) = e.compound("Properties") {
        let mut kv: Vec<String> = p.iter().map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or(""))).collect();
        kv.sort();
        s = format!("{s}[{}]", kv.join(","));
    }
    Some(s)
}
