//! Export Meadow worlds to Minecraft Java Edition (Anvil, 1.18.2 - 26.1).
//!
//! Regions are generated in parallel; each chunk is self-contained (objects that cross chunk
//! borders are evaluated by every chunk they touch), so there is no locking on the hot path.

pub mod chunk;
pub mod generate;
pub mod level;
pub mod region;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use meadow_blocks::McVersion;
use meadow_core::tile::TILE_SHIFT;
use meadow_core::{DimensionKind, World};
use parking_lot::Mutex;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

pub use generate::GenContext;

#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// The world folder to create (e.g. `.minecraft/saves/MyWorld`).
    pub dir: PathBuf,
    pub version: McVersion,
    pub dimensions: Vec<DimensionKind>,
    /// zlib level 1-9. 6 is Minecraft's own default; 1-3 export noticeably faster.
    pub compression: u32,
    /// Overwrite an existing folder.
    pub overwrite: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ExportStats {
    pub chunks: usize,
    pub regions: usize,
    pub elapsed: Duration,
    pub warnings: Vec<String>,
    pub datapacks: Vec<String>,
}

pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
    pub stage: Mutex<String>,
}

impl Default for Progress {
    fn default() -> Self {
        Progress { done: AtomicUsize::new(0), total: AtomicUsize::new(1), cancel: AtomicBool::new(false), stage: Mutex::new(String::new()) }
    }
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        self.done.load(Ordering::Relaxed) as f32 / self.total.load(Ordering::Relaxed).max(1) as f32
    }
}

fn region_dir(dir: &Path, kind: DimensionKind) -> PathBuf {
    match kind {
        DimensionKind::Overworld => dir.join("region"),
        DimensionKind::Nether => dir.join("DIM-1").join("region"),
        DimensionKind::End => dir.join("DIM1").join("region"),
    }
}

/// Chunks covered by tiles, grouped by region.
fn regions_of(world: &World, kind: DimensionKind) -> FxHashMap<(i32, i32), Vec<(i32, i32)>> {
    let mut out: FxHashMap<(i32, i32), Vec<(i32, i32)>> = FxHashMap::default();
    if let Some(d) = world.dimension(kind) {
        let per = 1 << (TILE_SHIFT - 4); // chunks per tile side
        let mut seen = FxHashSet::default();
        for t in d.tiles() {
            for cz in 0..per {
                for cx in 0..per {
                    let c = (t.tx * per + cx, t.ty * per + cz);
                    if seen.insert(c) {
                        out.entry((c.0 >> 5, c.1 >> 5)).or_default().push(c);
                    }
                }
            }
        }
    }
    out
}

pub fn export_world(world: &World, opts: &ExportOptions, progress: &Progress) -> Result<ExportStats> {
    let start = Instant::now();
    let dir = &opts.dir;
    if dir.exists() {
        if !opts.overwrite {
            bail!("{} already exists", dir.display());
        }
        for sub in ["region", "DIM-1", "DIM1", "datapacks"] {
            let p = dir.join(sub);
            if p.exists() {
                std::fs::remove_dir_all(&p).with_context(|| format!("removing old {}", p.display()))?;
            }
        }
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let mut stats = ExportStats::default();
    let jobs: Vec<(DimensionKind, FxHashMap<(i32, i32), Vec<(i32, i32)>>)> =
        opts.dimensions.iter().filter(|k| world.dimension(**k).is_some()).map(|k| (*k, regions_of(world, *k))).collect();
    let total: usize = jobs.iter().map(|(_, r)| r.values().map(Vec::len).sum::<usize>()).sum();
    progress.total.store(total.max(1), Ordering::Relaxed);
    progress.done.store(0, Ordering::Relaxed);

    let missing: Mutex<FxHashMap<String, u64>> = Mutex::new(FxHashMap::default());
    for (kind, regions) in &jobs {
        *progress.stage.lock() = format!("Exporting {}", kind.name());
        let dim = world.dimension(*kind).unwrap();
        let ctx = GenContext::new(world, dim);
        let rdir = region_dir(dir, *kind);
        std::fs::create_dir_all(&rdir)?;
        let mut list: Vec<(&(i32, i32), &Vec<(i32, i32)>)> = regions.iter().collect();
        list.sort();
        let results: Vec<Result<usize>> = list
            .par_iter()
            .map(|((rx, rz), chunks)| {
                let (rx, rz) = (*rx, *rz);
                let mut pal = chunk::PaletteCache::new(opts.version, world.export.replace_unavailable_blocks);
                let mut out: Vec<(i32, i32, Vec<u8>)> = chunks
                    .par_iter()
                    .filter_map(|&(cx, cz)| {
                        if progress.cancel.load(Ordering::Relaxed) {
                            return None;
                        }
                        let buf = ctx.chunk(cx, cz);
                        progress.done.fetch_add(1, Ordering::Relaxed);
                        buf.map(|b| (cx, cz, b))
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|(cx, cz, b)| {
                        let nbt = chunk::to_nbt(&b, &mut pal, &ctx.infos);
                        let data = meadow_nbt::to_zlib("", &nbt, opts.compression.clamp(1, 9)).expect("zlib");
                        (cx, cz, data)
                    })
                    .collect();
                if progress.cancel.load(Ordering::Relaxed) {
                    bail!("export cancelled");
                }
                out.sort_by_key(|c| (c.1, c.0));
                let n = out.len();
                if n > 0 {
                    region::write_region(&rdir.join(format!("r.{rx}.{rz}.mca")), &out)?;
                }
                let mut m = missing.lock();
                for (k, v) in pal.missing {
                    *m.entry(k).or_default() += v;
                }
                Ok(n)
            })
            .collect();
        for r in results {
            stats.chunks += r?;
            stats.regions += 1;
        }
    }

    *progress.stage.lock() = "Writing level.dat".into();
    let mut packs = Vec::new();
    let custom_height = !level::custom_height_dimensions(world).is_empty();
    if let Some(src) = &world.export.datapack {
        packs.push(level::install_datapack(dir, src)?);
    } else if custom_height {
        if world.export.generate_height_datapack && opts.version.can_generate_height_datapack() {
            if let Some(p) = level::write_height_datapack(dir, world, opts.version)? {
                packs.push(p);
            }
        } else {
            stats.warnings.push(format!(
                "This world uses build limits {}..{} but no data pack was bundled. Install the Higher Heights data pack \
                 (choose it under Export > Data pack) before opening the world in Minecraft {}.",
                world.limits.min_y,
                world.limits.max_y,
                opts.version.name()
            ));
        }
    }
    level::write_level_dat(dir, world, opts.version, &packs)?;
    write_icon(dir, world).ok();
    stats.datapacks = packs;
    for (name, n) in missing.into_inner() {
        stats.warnings.push(format!(
            "{name} does not exist in Minecraft {} ({n} palette entries){}",
            opts.version.name(),
            if world.export.replace_unavailable_blocks { "; replaced with a similar block" } else { "" }
        ));
    }
    stats.warnings.sort();
    stats.elapsed = start.elapsed();
    Ok(stats)
}

/// 64x64 map preview used as the world icon in Minecraft's world list.
fn write_icon(dir: &Path, world: &World) -> Result<()> {
    let d = world.surface();
    let Some((x0, z0, x1, z1)) = d.block_bounds() else { return Ok(()) };
    let size = 64u32;
    let step = ((x1 - x0).max(z1 - z0) as f32 / size as f32).max(1.0);
    let mut img = image::RgbImage::new(size, size);
    for (px, pz, p) in img.enumerate_pixels_mut() {
        let x = x0 + (px as f32 * step) as i32;
        let z = z0 + (pz as f32 * step) as i32;
        let c = match (d.terrain(x, z), d.height(x, z)) {
            (Some(t), Some(h)) => {
                let base = if d.is_underwater(x, z) { [52, 98, 200] } else { world.terrain(t).color };
                let shade = (d.height_or_min(x - 1, z - 1) - h).clamp(-4.0, 4.0) * -6.0;
                base.map(|v| (v as f32 + shade).clamp(0.0, 255.0) as u8)
            }
            _ => [0, 0, 0],
        };
        *p = image::Rgb(c);
    }
    img.save(dir.join("icon.png"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use meadow_blocks::BuildLimits;
    use meadow_core::layer::LayerId;
    use meadow_core::world::NewWorld;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("meadow-export-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        d
    }

    #[test]
    fn export_and_read_back() {
        let mut w = World::new(&NewWorld { width_tiles: 2, height_tiles: 2, base_height: 70.0, ..Default::default() });
        {
            let d = w.surface_mut();
            for z in -20..20 {
                for x in -20..20 {
                    d.set_layer_value(LayerId::DECIDUOUS, false, 0, x, z, 15);
                }
            }
            // A lake
            for z in 40..60 {
                for x in 40..60 {
                    d.set_height(x, z, 55.0);
                }
            }
        }
        let dir = tmp("basic");
        let opts = ExportOptions { dir: dir.clone(), version: McVersion::V1_21_4, dimensions: vec![DimensionKind::Overworld], compression: 3, overwrite: true };
        let stats = export_world(&w, &opts, &Progress::default()).unwrap();
        assert_eq!(stats.chunks, 16 * 16);
        assert!(dir.join("level.dat").exists());
        let chunks = region::read_region(&dir.join("region").join("r.0.0.mca")).unwrap();
        assert_eq!(chunks.len(), 8 * 8);
        // Chunk (2,2) contains block (40..48) -> lake
        let (_, _, c) = chunks.iter().find(|(x, z, _)| *x == 3 && *z == 3).unwrap();
        assert_eq!(c.i32("DataVersion"), Some(McVersion::V1_21_4.data_version()));
        assert_eq!(c.i32("yPos"), Some(-4));
        assert_eq!(chunk::decode_block(c, 2, 55, 2).as_deref(), Some("minecraft:dirt"));
        assert_eq!(chunk::decode_block(c, 2, 54, 2).as_deref(), Some("minecraft:dirt"));
        assert_eq!(chunk::decode_block(c, 2, 62, 2).as_deref(), Some("minecraft:water[level=0]"));
        assert_eq!(chunk::decode_block(c, 2, -64, 2).as_deref(), Some("minecraft:bedrock"));
        assert_eq!(chunk::decode_block(c, 2, -30, 2).as_deref(), Some("minecraft:deepslate[axis=y]"));
        // Trees around the origin: some log above ground in chunk (-1,-1) of region (-1,-1)
        let chunks = region::read_region(&dir.join("region").join("r.-1.-1.mca")).unwrap();
        let mut logs = 0;
        for (_, _, c) in &chunks {
            for y in 71..80 {
                for z in 0..16 {
                    for x in 0..16 {
                        if chunk::decode_block(c, x, y, z).is_some_and(|b| b.contains("_log")) {
                            logs += 1;
                        }
                    }
                }
            }
        }
        assert!(logs > 20, "logs: {logs}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tall_world_with_datapack_and_old_version() {
        let mut w = World::new(&NewWorld {
            width_tiles: 1,
            height_tiles: 1,
            limits: BuildLimits { min_y: -2032, max_y: 2032 },
            base_height: 1800.0,
            ..Default::default()
        });
        w.surface_mut().set_terrain(1, 1, meadow_core::terrain::ids::PALE_MOSS);
        let dir = tmp("tall");
        let opts = ExportOptions { dir: dir.clone(), version: McVersion::V1_20_1, dimensions: vec![DimensionKind::Overworld], compression: 1, overwrite: true };
        let stats = export_world(&w, &opts, &Progress::default()).unwrap();
        assert_eq!(stats.datapacks, vec!["file/meadow-build-height".to_string()]);
        let dt = std::fs::read_to_string(dir.join("datapacks/meadow-build-height/data/minecraft/dimension_type/overworld.json")).unwrap();
        assert!(dt.contains("\"height\": 4064") && dt.contains("\"min_y\": -2032"));
        // pale moss doesn't exist in 1.20.1 -> warning + replacement
        assert!(stats.warnings.iter().any(|w| w.contains("pale_moss_block")), "{:?}", stats.warnings);
        let chunks = region::read_region(&dir.join("region").join("r.0.0.mca")).unwrap();
        let (_, _, c) = chunks.iter().find(|(x, z, _)| *x == 0 && *z == 0).unwrap();
        assert_eq!(c.i32("yPos"), Some(-127));
        assert_eq!(c.list("sections").unwrap().len(), 254);
        assert_eq!(chunk::decode_block(c, 0, 1800, 0).as_deref(), Some("minecraft:grass_block[snowy=false]"));
        assert_eq!(chunk::decode_block(c, 1, 1800, 1).as_deref(), Some("minecraft:moss_block"));
        // short_grass is exported under its 1.20.1 name somewhere on the grass
        let mut found_grass = false;
        for z in 0..16 {
            for x in 0..16 {
                if chunk::decode_block(c, x, 1801, z).as_deref() == Some("minecraft:grass") {
                    found_grass = true;
                }
            }
        }
        assert!(found_grass);
        let level = std::fs::read(dir.join("level.dat")).unwrap();
        let (_, root) = meadow_nbt::from_any(&level).unwrap();
        let data = root.compound("Data").unwrap();
        assert_eq!(data.i32("DataVersion"), Some(3465));
        std::fs::remove_dir_all(dir).ok();
    }
}
