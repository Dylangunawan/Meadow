//! Import blocks (IDs, block-state properties, average texture colours) from a mod jar or the
//! Minecraft client jar. This is how Meadow picks up the exact Conquest Reforged block set the
//! user has installed, and accurate colours for any block.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};
use std::path::Path;

use anyhow::{Context, Result};
use rustc_hash::FxHashMap;
use serde_json::Value;

use crate::registry::{BlockInfo, BlockSource, PropDef, categorize, is_plant_like, registry};

#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub namespaces: Vec<(String, usize)>,
    pub blocks: usize,
    pub colored: usize,
}

/// Read a jar and return the blocks it defines.
pub fn read_jar(path: &Path) -> Result<Vec<BlockInfo>> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    read_zip(f)
}

pub fn read_zip<R: Read + Seek>(reader: R) -> Result<Vec<BlockInfo>> {
    let mut zip = zip::ZipArchive::new(reader).context("not a zip/jar file")?;
    let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    let mut texts: FxHashMap<String, String> = FxHashMap::default();
    let mut blockstates = Vec::new();
    for n in &names {
        let is_bs = n.starts_with("assets/") && n.contains("/blockstates/") && n.ends_with(".json");
        let is_model = n.starts_with("assets/") && n.contains("/models/") && n.ends_with(".json");
        if is_bs || is_model {
            let mut s = String::new();
            if zip.by_name(n)?.read_to_string(&mut s).is_ok() {
                texts.insert(n.clone(), s);
            }
            if is_bs {
                blockstates.push(n.clone());
            }
        }
    }
    let mut tex_cache: FxHashMap<String, Option<[u8; 4]>> = FxHashMap::default();
    let mut out = Vec::with_capacity(blockstates.len());
    for bs in blockstates {
        // assets/<ns>/blockstates/<path>.json
        let rest = &bs["assets/".len()..];
        let Some((ns, rest)) = rest.split_once('/') else { continue };
        let Some(path) = rest.strip_prefix("blockstates/").and_then(|p| p.strip_suffix(".json")) else { continue };
        let Ok(json) = serde_json::from_str::<Value>(&texts[&bs]) else { continue };
        let (props, model) = analyse_blockstate(&json);
        let color = model.and_then(|m| model_color(&m, &texts, &mut zip, &mut tex_cache)).map(|c| biome_tint(ns, path, c));
        let non_solid = is_plant_like(path);
        out.push(BlockInfo {
            id: format!("{ns}:{path}"),
            props,
            default: Vec::new(),
            since: 0,
            transparent: non_solid || color.is_some_and(|c| c[3] < 250),
            non_solid,
            light: 0,
            color: color.unwrap_or_else(|| crate::color::guess_color(path)),
            source: BlockSource::Imported,
            category: categorize(path),
        });
    }
    Ok(out)
}

/// Minecraft stores grass, leaves, vines and water textures in grey and tints them by biome at
/// run time. Apply the default (plains) tint so map colours look like the game.
pub fn biome_tint(ns: &str, path: &str, c: [u8; 4]) -> [u8; 4] {
    if ns != "minecraft" {
        return c;
    }
    let tint: u32 = match path {
        "water" | "bubble_column" | "water_cauldron" => 0x3F76E4,
        "birch_leaves" => 0x80A755,
        "spruce_leaves" => 0x619961,
        "lily_pad" => 0x208030,
        "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves" | "mangrove_leaves" | "vine" => 0x77AB2F,
        "grass_block" | "short_grass" | "grass" | "tall_grass" | "fern" | "large_fern" | "sugar_cane" | "potted_fern" => 0x91BD59,
        _ => return c,
    };
    // Only tint grey textures (don't tint twice if a resource pack already coloured them).
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    if mx - mn > 40 {
        return c;
    }
    let t = [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8];
    [0, 1, 2].map(|k| (c[k] as u32 * t[k] as u32 / 255) as u8).into_iter().chain([c[3]]).collect::<Vec<u8>>().try_into().unwrap()
}

/// Find the newest Minecraft release jar installed for the current user.
pub fn find_minecraft_jar() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    let candidates = [
        appdata.map(|a| a.join(".minecraft").join("versions")),
        home.as_ref().map(|h| h.join("Library/Application Support/minecraft/versions")),
        home.as_ref().map(|h| h.join(".minecraft").join("versions")),
    ];
    let mut best: Option<(Vec<u32>, std::path::PathBuf)> = None;
    for dir in candidates.into_iter().flatten() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // Releases only: "1.21.4", "26.1" (no snapshots, pre-releases or modded profiles).
            let parts: Option<Vec<u32>> = name.split('.').map(|p| p.parse().ok()).collect();
            let Some(parts) = parts else { continue };
            let jar = e.path().join(format!("{name}.jar"));
            if parts.len() >= 2 && jar.is_file() && best.as_ref().is_none_or(|(b, _)| parts > *b) {
                best = Some((parts, jar));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Merge imported blocks into the global registry. Vanilla blocks keep their generated state
/// definitions and only take the imported colour.
pub fn import_into_registry(blocks: Vec<BlockInfo>, colored_hint: &[bool]) -> ImportReport {
    let mut report = ImportReport::default();
    let mut ns_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut merged = Vec::with_capacity(blocks.len());
    {
        let reg = registry().read();
        for (i, b) in blocks.into_iter().enumerate() {
            *ns_counts.entry(b.namespace().to_owned()).or_default() += 1;
            if colored_hint.get(i).copied().unwrap_or(true) {
                report.colored += 1;
            }
            match reg.get(&b.id) {
                Some(existing) if existing.source == BlockSource::Vanilla => {
                    let mut e = (**existing).clone();
                    e.color = b.color;
                    merged.push(e);
                }
                _ => merged.push(b),
            }
        }
    }
    report.blocks = merged.len();
    registry().write().register_imported(merged);
    crate::state::refresh_infos();
    report.namespaces = ns_counts.into_iter().collect();
    report
}

/// Collect property names/values from `variants` keys or `multipart` conditions, and pick a
/// representative model.
fn analyse_blockstate(json: &Value) -> (Vec<PropDef>, Option<String>) {
    let mut props: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut model = None;
    if let Some(vars) = json.get("variants").and_then(Value::as_object) {
        for (key, v) in vars {
            for kv in key.split(',').filter(|s| !s.is_empty()) {
                if let Some((k, val)) = kv.split_once('=') {
                    props.entry(k.to_owned()).or_default().insert(val.to_owned());
                }
            }
            if model.is_none() {
                model = first_model(v);
            }
        }
    }
    if let Some(parts) = json.get("multipart").and_then(Value::as_array) {
        for p in parts {
            if let Some(w) = p.get("when") {
                collect_when(w, &mut props);
            }
            if model.is_none() {
                model = p.get("apply").and_then(first_model);
            }
        }
    }
    let defs = props
        .into_iter()
        .map(|(name, vals)| {
            let mut values: Vec<String> = vals.into_iter().collect();
            if values.iter().all(|v| v == "true" || v == "false") {
                values = vec!["true".into(), "false".into()];
            }
            PropDef { name, values }
        })
        .collect();
    (defs, model)
}

fn collect_when(w: &Value, props: &mut BTreeMap<String, BTreeSet<String>>) {
    if let Some(obj) = w.as_object() {
        for (k, v) in obj {
            if k == "OR" || k == "AND" {
                if let Some(arr) = v.as_array() {
                    for x in arr {
                        collect_when(x, props);
                    }
                }
            } else {
                let s = match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                for val in s.split('|') {
                    props.entry(k.clone()).or_default().insert(val.to_owned());
                }
            }
        }
    }
}

fn first_model(v: &Value) -> Option<String> {
    match v {
        Value::Array(a) => a.first().and_then(first_model),
        Value::Object(o) => o.get("model").and_then(Value::as_str).map(str::to_owned),
        _ => None,
    }
}

fn split_rl(rl: &str) -> (&str, &str) {
    rl.split_once(':').unwrap_or(("minecraft", rl))
}

fn model_color<R: Read + Seek>(
    model: &str,
    texts: &FxHashMap<String, String>,
    zip: &mut zip::ZipArchive<R>,
    cache: &mut FxHashMap<String, Option<[u8; 4]>>,
) -> Option<[u8; 4]> {
    // Gather textures following the parent chain (child entries win).
    let mut textures: BTreeMap<String, String> = BTreeMap::new();
    let mut current = Some(model.to_owned());
    for _ in 0..8 {
        let Some(m) = current.take() else { break };
        let (ns, p) = split_rl(&m);
        let key = format!("assets/{ns}/models/{p}.json");
        let Some(text) = texts.get(&key) else { break };
        let Ok(j) = serde_json::from_str::<Value>(text) else { break };
        if let Some(t) = j.get("textures").and_then(Value::as_object) {
            for (k, v) in t {
                if let Some(s) = v.as_str() {
                    textures.entry(k.clone()).or_insert_with(|| s.to_owned());
                }
            }
        }
        current = j.get("parent").and_then(Value::as_str).map(str::to_owned);
    }
    let pick = ["top", "all", "end", "texture", "side", "cross", "plant", "particle"]
        .iter()
        .find_map(|k| textures.get(*k).cloned())
        .or_else(|| textures.values().next().cloned())?;
    // Resolve #references
    let mut tex = pick;
    for _ in 0..8 {
        match tex.strip_prefix('#') {
            Some(r) => tex = textures.get(r)?.clone(),
            None => break,
        }
    }
    if let Some(c) = cache.get(&tex) {
        return *c;
    }
    let (ns, p) = split_rl(&tex);
    let path = format!("assets/{ns}/textures/{p}.png");
    let color = (|| {
        let mut buf = Vec::new();
        zip.by_name(&path).ok()?.read_to_end(&mut buf).ok()?;
        let img = image::load_from_memory_with_format(&buf, image::ImageFormat::Png).ok()?.to_rgba8();
        // Animated textures are vertical strips; the first frame is representative.
        let w = img.width();
        let h = img.height().min(w.max(1));
        let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for y in 0..h {
            for x in 0..w {
                let px = img.get_pixel(x, y).0;
                a += px[3] as u64;
                n += 1;
                if px[3] > 16 {
                    let wgt = px[3] as u64;
                    r += px[0] as u64 * wgt;
                    g += px[1] as u64 * wgt;
                    b += px[2] as u64 * wgt;
                }
            }
        }
        if a == 0 || n == 0 {
            return None;
        }
        Some([(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / n) as u8])
    })();
    cache.insert(tex, color);
    color
}

// ---------------------------------------------------------------------------------------------
// Persisting imported blocks (so a jar only has to be imported once).

pub fn blocks_to_tsv(blocks: &[BlockInfo]) -> String {
    let mut s = String::from("# Meadow imported blocks: id\tcolor\tflags\tproperties\n");
    for b in blocks {
        let props: Vec<String> = b.props.iter().map(|p| format!("{}:{}", p.name, p.values.join(","))).collect();
        let flags = format!("{}{}", if b.transparent { "t" } else { "" }, if b.non_solid { "e" } else { "" });
        s.push_str(&format!(
            "{}\t{:02x}{:02x}{:02x}{:02x}\t{}\t{}\n",
            b.id,
            b.color[0],
            b.color[1],
            b.color[2],
            b.color[3],
            if flags.is_empty() { "-" } else { &flags },
            if props.is_empty() { "-".to_string() } else { props.join(";") }
        ));
    }
    s
}

pub fn blocks_from_tsv(s: &str) -> Vec<BlockInfo> {
    s.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 4 {
                return None;
            }
            let rgba = u32::from_str_radix(c[1], 16).ok()?;
            let props = if c[3] == "-" {
                Vec::new()
            } else {
                c[3].split(';')
                    .filter_map(|p| {
                        let (n, v) = p.split_once(':')?;
                        Some(PropDef { name: n.into(), values: v.split(',').map(str::to_owned).collect() })
                    })
                    .collect()
            };
            let path = c[0].split_once(':').map(|x| x.1).unwrap_or(c[0]);
            Some(BlockInfo {
                id: c[0].to_owned(),
                props,
                default: Vec::new(),
                since: 0,
                transparent: c[2].contains('t'),
                non_solid: c[2].contains('e'),
                light: 0,
                color: rgba.to_be_bytes(),
                source: BlockSource::Imported,
                category: categorize(path),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn grey_grass_is_tinted_green() {
        let grey = [150, 150, 150, 255];
        let g = biome_tint("minecraft", "grass_block", grey);
        assert!(g[1] > g[0] && g[1] > g[2], "{g:?}");
        assert_eq!(biome_tint("minecraft", "stone", grey), grey);
        assert_eq!(biome_tint("conquest", "grass_block", grey), grey);
        // Already-coloured textures are left alone.
        assert_eq!(biome_tint("minecraft", "oak_leaves", [40, 120, 30, 255]), [40, 120, 30, 255]);
    }

    #[test]
    fn import_synthetic_jar() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opt = zip::write::SimpleFileOptions::default();
            z.start_file("assets/testmod/blockstates/red_thing_slab.json", opt).unwrap();
            z.write_all(br#"{"variants":{"type=bottom":{"model":"testmod:block/red_thing"},"type=top":{"model":"testmod:block/red_thing"}}}"#).unwrap();
            z.start_file("assets/testmod/models/block/red_thing.json", opt).unwrap();
            z.write_all(br#"{"parent":"block/cube_all","textures":{"all":"testmod:block/red"}}"#).unwrap();
            z.start_file("assets/testmod/textures/block/red.png", opt).unwrap();
            let mut png = Vec::new();
            let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 10, 10, 255]));
            img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            z.write_all(&png).unwrap();
            z.finish().unwrap();
        }
        buf.set_position(0);
        let blocks = read_zip(buf).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, "testmod:red_thing_slab");
        assert_eq!(blocks[0].props[0].values, vec!["bottom", "top"]);
        assert_eq!(blocks[0].color, [200, 10, 10, 255]);
        let tsv = blocks_to_tsv(&blocks);
        let back = blocks_from_tsv(&tsv);
        assert_eq!(back[0].color, [200, 10, 10, 255]);
        let rep = import_into_registry(back, &[]);
        assert_eq!(rep.blocks, 1);
        assert!(registry().read().get("testmod:red_thing_slab").is_some());
    }
}
