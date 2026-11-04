//! meadow-cli: headless Meadow tools.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use meadow_blocks::{BuildLimits, McVersion};
use meadow_core::heightmap::{Channel, HeightMap, ImportSettings, import_into};
use meadow_core::world::NewWorld;
use meadow_core::{DimensionKind, World};

const USAGE: &str = "\
meadow-cli — headless Meadow tools

USAGE:
  meadow-cli script <script.js> [--world <in.meadow>] [--save <out.meadow>] [--param name=value]... [-- args...]
      Run a WorldPainter-compatible script (optionally on a world, which becomes `world`).
  meadow-cli import <heightmap.png|.tif|.r16> <out.meadow> [--scale 100] [--image-range LO HI]
                    [--height-range LO HI] [--limits MIN MAX] [--water 62] [--name NAME]
      Create a world from a height map.
  meadow-cli export <world.meadow> <saves-dir/WorldName> [--version 26.1] [--compression 6] [--datapack pack.zip]
      Export to a Minecraft Java world.
  meadow-cli render <world.meadow> <out.png> [--size 1600x900] [--distance D] [--pitch DEG] [--yaw DEG]
      Render the 3D view to an image (works without a window).
  meadow-cli new <out.meadow> [--tiles 8] [--hills 40] [--limits MIN MAX]
      Create a new world.
  meadow-cli info <world.meadow>
  meadow-cli blocks [query] [--ns minecraft|conquest|...]
  meadow-cli versions
";

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn arg_pair(args: &[String], name: &str) -> Option<(f32, f32)> {
    let i = args.iter().position(|a| a == name)?;
    Some((args.get(i + 1)?.parse().ok()?, args.get(i + 2)?.parse().ok()?))
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("script") => cmd_script(&args[1..]),
        Some("import") => cmd_import(&args[1..]),
        Some("export") => cmd_export(&args[1..]),
        Some("render") => cmd_render(&args[1..]),
        Some("new") => cmd_new(&args[1..]),
        Some("info") => cmd_info(&args[1..]),
        Some("blocks") => cmd_blocks(&args[1..]),
        Some("versions") => {
            for v in McVersion::ALL {
                println!("{:8} data version {:5}  pack format {:3}  {}", v.name(), v.data_version(), v.pack_format(), v.label());
            }
            Ok(())
        }
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    };
    if let Err(e) = r {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn cmd_script(args: &[String]) -> Result<()> {
    let path = PathBuf::from(args.first().context("missing script path")?);
    let source = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let world = match arg_value(args, "--world") {
        Some(w) => Some(World::load(Path::new(&w))?),
        None => None,
    };
    let mut params = BTreeMap::new();
    let desc = meadow_script::parse_descriptor(&source);
    for p in &desc.params {
        if let Some(d) = &p.default {
            params.insert(p.name.clone(), parse_param(d));
        }
    }
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--param"
            && let Some(kv) = args.get(i + 1)
            && let Some((k, v)) = kv.split_once('=')
        {
            params.insert(k.to_owned(), parse_param(v));
        }
        i += 1;
    }
    let script_args: Vec<String> = args.iter().skip_while(|a| *a != "--").skip(1).cloned().collect();
    let t = Instant::now();
    let r = meadow_script::run_script(&source, &path, world, &script_args, &params, Arc::new(AtomicBool::new(false)), |s| print!("{s}"));
    if let Some(e) = &r.error {
        bail!("script failed: {e}");
    }
    eprintln!("script finished in {:.2}s (exit code {})", t.elapsed().as_secs_f32(), r.exit_code);
    if let (Some(out), Some(mut w)) = (arg_value(args, "--save"), r.world) {
        w.save(Path::new(&out))?;
        eprintln!("saved {out}");
    }
    if r.exit_code != 0 {
        std::process::exit(r.exit_code);
    }
    Ok(())
}

fn parse_param(v: &str) -> meadow_script::ParamValue {
    use meadow_script::ParamValue as P;
    if let Ok(i) = v.parse::<i64>() {
        P::Int(i)
    } else if let Ok(f) = v.parse::<f64>() {
        P::Float(f)
    } else if v == "true" || v == "false" {
        P::Bool(v == "true")
    } else {
        P::Str(v.to_owned())
    }
}

fn limits_arg(args: &[String]) -> BuildLimits {
    arg_pair(args, "--limits").map(|(a, b)| BuildLimits { min_y: a as i32, max_y: b as i32 }.sanitized()).unwrap_or(BuildLimits::VANILLA)
}

fn cmd_import(args: &[String]) -> Result<()> {
    let src = PathBuf::from(args.first().context("missing height map")?);
    let out = PathBuf::from(args.get(1).context("missing output .meadow")?);
    let limits = limits_arg(args);
    let map = HeightMap::load(&src, Channel::Grey)?;
    let mut s = ImportSettings::for_map(&map, limits.min_y, limits.max_y);
    if let Some(v) = arg_value(args, "--scale") {
        s.scale_percent = v.parse()?;
    }
    if let Some((a, b)) = arg_pair(args, "--image-range") {
        s.image_low = a;
        s.image_high = b;
    }
    if let Some((a, b)) = arg_pair(args, "--height-range") {
        s.world_low = a;
        s.world_high = b;
    }
    if let Some(w) = arg_value(args, "--water") {
        s.water_level = w.parse()?;
    }
    let (w, h) = s.world_size(&map);
    s.offset = (-w / 2, -h / 2);
    let t = Instant::now();
    let name = arg_value(args, "--name").unwrap_or_else(|| src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let mut world = World::new(&NewWorld { name, width_tiles: 0, height_tiles: 0, limits, water_level: s.water_level, ..Default::default() });
    let d = world.surface_mut();
    d.default_water_level = s.water_level;
    let n = import_into(d, &map, &s);
    d.clear_history();
    eprintln!("imported {}x{} px -> {w}x{h} blocks ({n} tiles) in {:.2}s", map.width, map.height, t.elapsed().as_secs_f32());
    world.save(&out)?;
    eprintln!("saved {}", out.display());
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<()> {
    let src = PathBuf::from(args.first().context("missing world")?);
    let dir = PathBuf::from(args.get(1).context("missing output directory")?);
    let mut world = World::load(&src)?;
    let version = match arg_value(args, "--version") {
        Some(v) => McVersion::from_name(&v).with_context(|| format!("unknown version {v} (see `meadow-cli versions`)"))?,
        None => world.export.version,
    };
    if let Some(p) = arg_value(args, "--datapack") {
        world.export.datapack = Some(PathBuf::from(p));
    }
    let compression = arg_value(args, "--compression").map(|c| c.parse()).transpose()?.unwrap_or(6);
    let opts = meadow_export::ExportOptions {
        dir: dir.clone(),
        version,
        dimensions: world.dimensions.iter().map(|d| d.kind).collect(),
        compression,
        overwrite: true,
    };
    let progress = meadow_export::Progress::default();
    let stats = meadow_export::export_world(&world, &opts, &progress)?;
    println!(
        "exported {} chunks in {} regions to {} for Minecraft {} in {:.2}s ({:.0} chunks/s)",
        stats.chunks,
        stats.regions,
        dir.display(),
        version.name(),
        stats.elapsed.as_secs_f32(),
        stats.chunks as f32 / stats.elapsed.as_secs_f32().max(0.001)
    );
    for d in &stats.datapacks {
        println!("data pack: {d}");
    }
    for w in &stats.warnings {
        println!("warning: {w}");
    }
    Ok(())
}

fn cmd_render(args: &[String]) -> Result<()> {
    let src = PathBuf::from(args.first().context("missing world")?);
    let out = PathBuf::from(args.get(1).context("missing output image")?);
    let (w, h) = arg_value(args, "--size")
        .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
        .unwrap_or((1600u32, 900u32));
    let world = Arc::new(World::load(&src)?);
    let r = meadow_render::Renderer::headless()?;
    let mut scene = meadow_render::Scene::new(&r);
    scene.set_world(world, DimensionKind::Overworld);
    scene.frame_world();
    if let Some(d) = arg_value(args, "--distance") {
        scene.camera.distance = d.parse()?;
    }
    if let Some(p) = arg_value(args, "--pitch") {
        scene.camera.pitch = p.parse::<f32>()?.to_radians();
    }
    if let Some(y) = arg_value(args, "--yaw") {
        scene.camera.yaw = y.parse::<f32>()?.to_radians();
    }
    let t = Instant::now();
    scene.finish_loading(&r, w as f32 / h as f32);
    let meshing = t.elapsed();
    scene.render(&r, w, h);
    let img = scene.read_pixels(&r).context("readback failed")?;
    img.save(&out)?;
    eprintln!(
        "rendered {} tiles, {:.2}M triangles (meshing {:.2}s) -> {}",
        scene.stats.tiles_drawn,
        scene.stats.triangles as f64 / 1e6,
        meshing.as_secs_f32(),
        out.display()
    );
    Ok(())
}

fn cmd_new(args: &[String]) -> Result<()> {
    let out = PathBuf::from(args.first().context("missing output .meadow")?);
    let tiles: i32 = arg_value(args, "--tiles").map(|t| t.parse()).transpose()?.unwrap_or(8);
    let hills: f32 = arg_value(args, "--hills").map(|t| t.parse()).transpose()?.unwrap_or(40.0);
    let limits = limits_arg(args);
    let base = if limits.max_y > 1000 { (limits.min_y + limits.max_y) as f32 / 2.0 + 200.0 } else { 70.0 };
    let mut w = World::new(&NewWorld { width_tiles: tiles, height_tiles: tiles, hills, limits, base_height: base, ..Default::default() });
    w.save(&out)?;
    eprintln!("created {} ({} tiles)", out.display(), tiles * tiles);
    Ok(())
}

fn cmd_info(args: &[String]) -> Result<()> {
    let src = PathBuf::from(args.first().context("missing world")?);
    let w = World::load(&src)?;
    println!("name:    {}", w.name);
    println!("seed:    {}", w.seed);
    println!("limits:  {}..{} ({} blocks)", w.limits.min_y, w.limits.max_y, w.limits.height());
    println!("export:  {}", w.export.version.label());
    for d in &w.dimensions {
        let (lo, hi) = d.tiles().fold((f32::MAX, f32::MIN), |(a, b), t| (a.min(t.min_height()), b.max(t.max_height())));
        println!("{:8} {} tiles, heights {:.0}..{:.0}, bounds {:?}", d.kind.name(), d.tile_count(), lo, hi, d.block_bounds());
    }
    let custom: Vec<&str> = w.layers.iter().filter(|l| l.id.is_custom()).map(|l| l.name.as_str()).collect();
    println!("custom layers: {}", if custom.is_empty() { "none".into() } else { custom.join(", ") });
    println!("masks: {}", w.masks.len());
    Ok(())
}

fn cmd_blocks(args: &[String]) -> Result<()> {
    let ns = arg_value(args, "--ns");
    let q: Vec<&str> = args.iter().take_while(|a| !a.starts_with("--")).map(String::as_str).collect();
    let reg = meadow_blocks::registry().read();
    let res = reg.search(&q.join(" "), ns.as_deref(), 200);
    for b in &res {
        println!("{:60} {:18} #{:02x}{:02x}{:02x}", b.id, b.category, b.color[0], b.color[1], b.color[2]);
    }
    eprintln!("{} shown; registry has {} blocks in namespaces {:?}", res.len(), reg.len(), reg.namespaces());
    Ok(())
}
