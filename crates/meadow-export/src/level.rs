//! `level.dat` and data packs (build height overrides / bundled Higher Heights pack).

use std::path::Path;

use anyhow::{Context, Result};
use meadow_blocks::{BuildLimits, McVersion};
use meadow_core::world::{Generator, GameMode};
use meadow_core::{DimensionKind, World};
use meadow_nbt::{Compound, List, Tag};

pub const HEIGHT_PACK_NAME: &str = "meadow-build-height";

fn vanilla_limits(kind: DimensionKind) -> BuildLimits {
    match kind {
        DimensionKind::Overworld => BuildLimits::VANILLA,
        _ => BuildLimits { min_y: 0, max_y: 256 },
    }
}

/// Dimensions whose build limits differ from vanilla.
pub fn custom_height_dimensions(world: &World) -> Vec<DimensionKind> {
    world.dimensions.iter().filter(|d| d.limits != vanilla_limits(d.kind)).map(|d| d.kind).collect()
}

fn generator_tag(world: &World, kind: DimensionKind, version: McVersion) -> Compound {
    let painted = world.dimension(kind).is_some();
    let generator = if painted { world.export.generator } else { Generator::Default };
    let flat = |layers: Vec<(&str, i32)>, biome: &str| {
        let mut l = List::new();
        for (b, h) in layers {
            l.push(Compound::new().with("block", b).with("height", h));
        }
        let mut settings = Compound::new().with("layers", l).with("biome", biome).with("features", 0i8).with("lakes", 0i8);
        if version >= McVersion::V1_19_4 {
            settings.insert("structure_overrides", List::new());
        } else {
            settings.insert("structures", Compound::new().with("structures", Compound::new()));
        }
        Compound::new().with("type", "minecraft:flat").with("settings", settings)
    };
    match (generator, kind) {
        (Generator::Void, _) => flat(vec![("minecraft:air", 1)], "minecraft:the_void"),
        (Generator::Flat, DimensionKind::Overworld) => {
            flat(vec![("minecraft:bedrock", 1), ("minecraft:dirt", 2), ("minecraft:grass_block", 1)], "minecraft:plains")
        }
        (_, DimensionKind::Overworld) => Compound::new()
            .with("type", "minecraft:noise")
            .with("settings", "minecraft:overworld")
            .with("biome_source", Compound::new().with("type", "minecraft:multi_noise").with("preset", "minecraft:overworld")),
        (_, DimensionKind::Nether) => Compound::new()
            .with("type", "minecraft:noise")
            .with("settings", "minecraft:nether")
            .with("biome_source", Compound::new().with("type", "minecraft:multi_noise").with("preset", "minecraft:nether")),
        (_, DimensionKind::End) => Compound::new()
            .with("type", "minecraft:noise")
            .with("settings", "minecraft:end")
            .with("biome_source", Compound::new().with("type", "minecraft:the_end")),
    }
}

pub fn write_level_dat(dir: &Path, world: &World, version: McVersion, datapacks: &[String]) -> Result<()> {
    let surface = world.surface();
    let (sx, sz) = world.spawn;
    let sy = surface.int_height(sx, sz).map(|h| h + 1).unwrap_or(64).max(surface.water(sx, sz).unwrap_or(i32::MIN) + 1);
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let mut dims = Compound::new();
    for (kind, key, ty) in [
        (DimensionKind::Overworld, "minecraft:overworld", "minecraft:overworld"),
        (DimensionKind::Nether, "minecraft:the_nether", "minecraft:the_nether"),
        (DimensionKind::End, "minecraft:the_end", "minecraft:the_end"),
    ] {
        dims.insert(key, Compound::new().with("type", ty).with("generator", generator_tag(world, kind, version)));
    }
    let wgs = Compound::new()
        .with("seed", world.seed as i64)
        .with("generate_features", (world.export.generator == Generator::Default) as i8)
        .with("bonus_chest", 0i8)
        .with("dimensions", dims);
    let mut enabled = List::new();
    enabled.push(Tag::from("vanilla"));
    for d in datapacks {
        enabled.push(Tag::from(d.as_str()));
    }
    let game_type = match world.export.game_mode {
        GameMode::Survival => 0,
        GameMode::Creative => 1,
        GameMode::Adventure => 2,
        GameMode::Spectator => 3,
    };
    let data = Compound::new()
        .with("DataVersion", version.data_version())
        .with("version", 19133i32)
        .with(
            "Version",
            Compound::new()
                .with("Id", version.data_version())
                .with("Name", version.name())
                .with("Series", "main")
                .with("Snapshot", 0i8),
        )
        .with("LevelName", world.name.as_str())
        .with("GameType", game_type)
        .with("allowCommands", world.export.allow_cheats as i8)
        .with("Difficulty", 2i8)
        .with("hardcore", 0i8)
        .with("initialized", 1i8)
        .with("LastPlayed", now_ms)
        .with("SpawnX", sx)
        .with("SpawnY", sy)
        .with("SpawnZ", sz)
        .with("SpawnAngle", 0f32)
        .with(
            "spawn",
            Compound::new()
                .with("dimension", "minecraft:overworld")
                .with("pos", vec![sx, sy, sz])
                .with("yaw", 0f32)
                .with("pitch", 0f32),
        )
        .with("Time", 0i64)
        .with("DayTime", 6000i64)
        .with("raining", 0i8)
        .with("thundering", 0i8)
        .with("rainTime", 100000i32)
        .with("thunderTime", 100000i32)
        .with("clearWeatherTime", 0i32)
        .with("WorldGenSettings", wgs)
        .with("DataPacks", Compound::new().with("Enabled", enabled).with("Disabled", List::new()))
        .with("WasModded", 0i8)
        .with("ServerBrands", List::of(vec![Tag::from("Meadow")]));
    let root = Compound::new().with("Data", data);
    let bytes = meadow_nbt::to_gzip("", &root)?;
    std::fs::write(dir.join("level.dat"), bytes).context("writing level.dat")?;
    Ok(())
}

fn dimension_type_json(kind: DimensionKind, limits: BuildLimits, version: McVersion) -> serde_json::Value {
    use serde_json::json;
    let logical = match kind {
        DimensionKind::Nether => 128.min(limits.height()),
        _ => limits.height(),
    };
    let mut v = match kind {
        DimensionKind::Overworld => json!({
            "ultrawarm": false, "natural": true, "coordinate_scale": 1.0, "has_skylight": true,
            "has_ceiling": false, "ambient_light": 0.0, "piglin_safe": false, "bed_works": true,
            "respawn_anchor_works": false, "has_raids": true, "infiniburn": "#minecraft:infiniburn_overworld",
            "effects": "minecraft:overworld",
        }),
        DimensionKind::Nether => json!({
            "ultrawarm": true, "natural": false, "coordinate_scale": 8.0, "has_skylight": false,
            "has_ceiling": true, "ambient_light": 0.1, "fixed_time": 18000, "piglin_safe": true,
            "bed_works": false, "respawn_anchor_works": true, "has_raids": false,
            "infiniburn": "#minecraft:infiniburn_nether", "effects": "minecraft:the_nether",
        }),
        DimensionKind::End => json!({
            "ultrawarm": false, "natural": false, "coordinate_scale": 1.0, "has_skylight": false,
            "has_ceiling": false, "ambient_light": 0.0, "fixed_time": 6000, "piglin_safe": false,
            "bed_works": false, "respawn_anchor_works": false, "has_raids": true,
            "infiniburn": "#minecraft:infiniburn_end", "effects": "minecraft:the_end",
        }),
    };
    let o = v.as_object_mut().unwrap();
    o.insert("min_y".into(), json!(limits.min_y));
    o.insert("height".into(), json!(limits.height()));
    o.insert("logical_height".into(), json!(logical));
    if version >= McVersion::V1_19_4 {
        match kind {
            DimensionKind::Nether => {
                o.insert("monster_spawn_light_level".into(), json!(7));
                o.insert("monster_spawn_block_light_limit".into(), json!(15));
            }
            _ => {
                o.insert(
                    "monster_spawn_light_level".into(),
                    json!({"type": "minecraft:uniform", "min_inclusive": 0, "max_inclusive": 7}),
                );
                o.insert("monster_spawn_block_light_limit".into(), json!(0));
            }
        }
    }
    v
}

/// Write Meadow's own build height data pack (folder pack). Returns its pack id.
pub fn write_height_datapack(dir: &Path, world: &World, version: McVersion) -> Result<Option<String>> {
    let kinds = custom_height_dimensions(world);
    if kinds.is_empty() {
        return Ok(None);
    }
    let root = dir.join("datapacks").join(HEIGHT_PACK_NAME);
    let types = root.join("data").join("minecraft").join("dimension_type");
    std::fs::create_dir_all(&types)?;
    let fmt = version.pack_format();
    let mut pack = serde_json::json!({ "description": "Build height set by Meadow", "pack_format": fmt });
    if fmt >= 18 {
        pack["supported_formats"] = serde_json::json!([fmt, fmt]);
    }
    if fmt >= 82 {
        pack["min_format"] = serde_json::json!(fmt);
        pack["max_format"] = serde_json::json!(fmt);
    }
    std::fs::write(root.join("pack.mcmeta"), serde_json::to_string_pretty(&serde_json::json!({ "pack": pack }))?)?;
    for kind in kinds {
        let d = world.dimension(kind).unwrap();
        let name = match kind {
            DimensionKind::Overworld => "overworld",
            DimensionKind::Nether => "the_nether",
            DimensionKind::End => "the_end",
        };
        let json = dimension_type_json(kind, d.limits, version);
        std::fs::write(types.join(format!("{name}.json")), serde_json::to_string_pretty(&json)?)?;
    }
    Ok(Some(format!("file/{HEIGHT_PACK_NAME}")))
}

/// Copy a user-supplied data pack (zip or folder) into the world. Returns its pack id.
pub fn install_datapack(dir: &Path, src: &Path) -> Result<String> {
    let packs = dir.join("datapacks");
    std::fs::create_dir_all(&packs)?;
    let name = src.file_name().context("data pack path has no file name")?.to_string_lossy().into_owned();
    if src.is_dir() {
        copy_dir(src, &packs.join(&name))?;
    } else {
        std::fs::copy(src, packs.join(&name)).with_context(|| format!("copying {}", src.display()))?;
    }
    Ok(format!("file/{name}"))
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &dst.join(e.file_name()))?;
        } else {
            std::fs::copy(&p, dst.join(e.file_name()))?;
        }
    }
    Ok(())
}
