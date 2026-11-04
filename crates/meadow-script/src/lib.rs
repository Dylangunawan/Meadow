//! WorldPainter-compatible scripting. Scripts written for WorldPainter (JavaScript run by
//! Nashorn) run on an embedded QuickJS engine with a `wp` object that mirrors WorldPainter's
//! scripting API, plus the Java-style `world`/`dimension` objects, `params`, `arguments`,
//! `print`, `Java.type(...)` and `org.pepsoft...` shims that existing scripts rely on.

mod descriptor;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use meadow_blocks::{BuildLimits, McVersion};
use meadow_core::dimension::DimensionKind;
use meadow_core::heightmap::{Channel, HeightMap, ImportSettings, Resample, import_into};
use meadow_core::layer::{DataSize, LayerId};
use meadow_core::mask::{CustomMask, MaskNode};
use meadow_core::terrain::{custom_slot_index, ids};
use meadow_core::{NewWorld, World};
use rquickjs::{Context, Ctx, Exception, Function, Object, Runtime, Value};
use serde_json::Value as Json;

pub use descriptor::{ParamType, ScriptDescriptor, ScriptParam, parse_descriptor};

const PRELUDE: &str = include_str!("prelude.js");

/// A parameter value passed to a script (`params['name']`).
#[derive(Debug, Clone)]
pub enum ParamValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Null,
}

pub struct ScriptOutcome {
    /// The current world (as passed in, modified by the script).
    pub world: Option<World>,
    /// Worlds the script created or loaded, if it did not save them itself.
    pub other_worlds: Vec<World>,
    pub error: Option<String>,
    pub exit_code: i32,
}

struct Host {
    worlds: Vec<Option<World>>,
    heightmaps: Vec<HeightMap>,
    filters: Vec<CustomMask>,
    out: Box<dyn FnMut(&str)>,
    script_dir: PathBuf,
}

impl Host {
    fn world(&mut self, id: f64) -> Result<&mut World, String> {
        self.worlds.get_mut(id as usize).and_then(|w| w.as_mut()).ok_or_else(|| format!("no world with id {id}"))
    }
    fn dim(&mut self, w: f64, d: f64) -> Result<&mut meadow_core::Dimension, String> {
        let kind = DimensionKind::from_index(d as i32).ok_or_else(|| format!("invalid dimension {d}"))?;
        self.world(w)?.dimension_mut(kind).ok_or_else(|| format!("dimension {d} does not exist"))
    }
    fn resolve(&self, p: &str) -> PathBuf {
        let path = Path::new(p);
        if path.is_absolute() {
            return path.to_owned();
        }
        // Prefer the script's folder; fall back to the working directory only for files that
        // exist there (WorldPainter resolved against the working directory).
        let in_dir = self.script_dir.join(path);
        if in_dir.exists() {
            return in_dir;
        }
        match std::env::current_dir().map(|c| c.join(path)) {
            Ok(cwd) if cwd.exists() => cwd,
            _ => in_dir,
        }
    }
}

fn throw(c: &Ctx<'_>, msg: impl Display) -> rquickjs::Error {
    Exception::throw_message(c, &msg.to_string())
}

/// Run a script. `world` becomes the `world`/`dimension` globals (as in WorldPainter's script
/// runner); the whole run is a single undo step on it.
pub fn run_script(
    source: &str,
    script_path: &Path,
    world: Option<World>,
    args: &[String],
    params: &BTreeMap<String, ParamValue>,
    cancel: Arc<AtomicBool>,
    out: impl FnMut(&str) + 'static,
) -> ScriptOutcome {
    let has_world = world.is_some();
    let mut worlds = Vec::new();
    if let Some(mut w) = world {
        for d in &mut w.dimensions {
            d.begin_op("Script");
        }
        worlds.push(Some(w));
    }
    let host = Rc::new(RefCell::new(Host {
        worlds,
        heightmaps: Vec::new(),
        filters: Vec::new(),
        out: Box::new(out),
        script_dir: script_path.parent().map(Path::to_owned).unwrap_or_default(),
    }));
    let mut error = None;
    let mut exit_code = 0;
    match Runtime::new().and_then(|rt| Context::full(&rt).map(|c| (rt, c))) {
        Err(e) => error = Some(format!("could not start the script engine: {e}")),
        Ok((rt, context)) => {
            let flag = cancel.clone();
            rt.set_interrupt_handler(Some(Box::new(move || flag.load(Ordering::Relaxed))));
            context.with(|ctx| {
                if let Err(e) = install(&ctx, &host) {
                    error = Some(format!("could not initialise scripting API: {e}"));
                    return;
                }
                if let Err(e) = set_globals(&ctx, script_path, has_world, args, params) {
                    error = Some(format!("could not set script globals: {e}"));
                    return;
                }
                if let Err(e) = ctx.eval::<(), _>(PRELUDE) {
                    error = Some(if cancel.load(Ordering::Relaxed) {
                        "Script cancelled".into()
                    } else {
                        format!("prelude failed: {}", describe(&ctx, e))
                    });
                    return;
                }
                if has_world {
                    let _ = ctx.eval::<(), _>("var world = new __World(0); var dimension = world.getDimension(0);");
                }
                let result = ctx.eval::<Value, _>(source.to_owned());
                if let Err(e) = result {
                    let v = ctx.catch();
                    if let Some(o) = v.as_object()
                        && let Ok(code) = o.get::<_, f64>("__exit")
                    {
                        exit_code = code as i32;
                        return;
                    }
                    error = Some(if cancel.load(Ordering::Relaxed) { "Script cancelled".into() } else { describe_value(&v, e) });
                }
            });
        }
    }
    let mut h = match Rc::try_unwrap(host) {
        Ok(h) => h.into_inner(),
        Err(rc) => {
            let mut b = rc.borrow_mut();
            Host {
                worlds: std::mem::take(&mut b.worlds),
                heightmaps: Vec::new(),
                filters: Vec::new(),
                out: Box::new(|_| {}),
                script_dir: PathBuf::new(),
            }
        }
    };
    let mut ws = std::mem::take(&mut h.worlds).into_iter();
    let world = if has_world {
        ws.next().flatten().map(|mut w| {
            for d in &mut w.dimensions {
                d.end_op();
            }
            w.dirty = true;
            w
        })
    } else {
        None
    };
    ScriptOutcome { world, other_worlds: ws.flatten().collect(), error, exit_code }
}

fn describe(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    let v = ctx.catch();
    describe_value(&v, e)
}

fn describe_value(v: &Value<'_>, e: rquickjs::Error) -> String {
    if let Some(ex) = v.as_exception() {
        let msg = ex.message().unwrap_or_default();
        let stack = ex.stack().unwrap_or_default();
        return if stack.is_empty() { msg } else { format!("{msg}\n{stack}") };
    }
    if let Some(s) = v.as_string().and_then(|s| s.to_string().ok()) {
        return s;
    }
    e.to_string()
}

fn set_globals(ctx: &Ctx<'_>, path: &Path, _has_world: bool, args: &[String], params: &BTreeMap<String, ParamValue>) -> rquickjs::Result<()> {
    let g = ctx.globals();
    let name = path.display().to_string();
    let arr = rquickjs::Array::new(ctx.clone())?;
    for (i, a) in args.iter().enumerate() {
        arr.set(i, a.as_str())?;
    }
    g.set("arguments", arr.clone())?;
    let argv = rquickjs::Array::new(ctx.clone())?;
    argv.set(0, name.as_str())?;
    for (i, a) in args.iter().enumerate() {
        argv.set(i + 1, a.as_str())?;
    }
    g.set("argv", argv)?;
    g.set("argc", args.len() as i32 + 1)?;
    g.set("scriptDir", path.parent().map(|p| p.display().to_string()).unwrap_or_default())?;
    let p = Object::new(ctx.clone())?;
    for (k, v) in params {
        match v {
            ParamValue::Int(i) => p.set(k.as_str(), *i as f64)?,
            ParamValue::Float(f) => p.set(k.as_str(), *f)?,
            ParamValue::Bool(b) => p.set(k.as_str(), *b)?,
            ParamValue::Str(s) => p.set(k.as_str(), s.as_str())?,
            ParamValue::Null => p.set(k.as_str(), rquickjs::Null)?,
        }
    }
    g.set("params", p)?;
    let ds = Object::new(ctx.clone())?;
    for n in ["BIT", "NIBBLE", "BYTE", "BIT_PER_CHUNK", "NONE"] {
        ds.set(n, n)?;
    }
    g.set("DataSize", ds)?;
    Ok(())
}

fn data_size_name(d: DataSize) -> &'static str {
    match d {
        DataSize::Bit => "BIT",
        DataSize::Nibble => "NIBBLE",
        DataSize::Byte => "BYTE",
    }
}

fn terrain_key(name: &str) -> String {
    name.to_uppercase().replace([' ', '-'], "_").replace("STAINED_CLAY", "STAINED_CLAY").replace("GRAY", "GREY")
}

#[allow(clippy::too_many_lines)]
fn install<'js>(ctx: &Ctx<'js>, host: &Rc<RefCell<Host>>) -> rquickjs::Result<()> {
    let m = Object::new(ctx.clone())?;
    macro_rules! native {
        ($name:literal, |$h:ident, $c:ident $(, $a:ident : $t:ty)*| -> $ret:ty $body:block) => {{
            let host = host.clone();
            m.set(
                $name,
                Function::new(ctx.clone(), move |$c: Ctx<'js> $(, $a: $t)*| -> $ret {
                    let _ = &$c;
                    let mut guard = host.borrow_mut();
                    #[allow(unused_variables)]
                    let $h: &mut Host = &mut guard;
                    $body
                })?,
            )?;
        }};
    }
    macro_rules! tri {
        ($c:ident, $e:expr) => {
            match $e {
                Ok(v) => v,
                Err(err) => return Err(throw(&$c, err)),
            }
        };
    }

    native!("version", |h, c| -> rquickjs::Result<String> { Ok(format!("Meadow {} (WorldPainter script compatible)", env!("CARGO_PKG_VERSION"))) });
    native!("print", |h, c, s: String| -> rquickjs::Result<()> {
        (h.out)(&format!("{s}\n"));
        Ok(())
    });
    native!("printRaw", |h, c, s: String| -> rquickjs::Result<()> {
        (h.out)(&s);
        Ok(())
    });
    native!("fileExists", |h, c, p: String| -> rquickjs::Result<bool> { Ok(h.resolve(&p).exists()) });
    native!("terrains", |h, c| -> rquickjs::Result<String> {
        let list: Vec<Json> = meadow_core::terrain::builtin_terrains()
            .iter()
            .map(|t| {
                let key = if t.custom { format!("CUSTOM_{}", t.name.trim_start_matches("Custom ")) } else { terrain_key(&t.name) };
                serde_json::json!({"key": key, "name": t.name, "index": t.id})
            })
            .collect();
        Ok(Json::Array(list).to_string())
    });
    native!("customSlotIndex", |h, c, i: f64| -> rquickjs::Result<i32> { Ok(custom_slot_index(i as usize).map(|v| v as i32).unwrap_or(-1)) });
    native!("layerByName", |h, c, w: f64, name: String| -> rquickjs::Result<Option<String>> {
        let defs = if w >= 0.0 {
            h.world(w).ok().map(|w| w.layers.clone())
        } else {
            h.worlds.iter().flatten().next().map(|w| w.layers.clone())
        }
        .unwrap_or_else(meadow_core::layer::builtin_layers);
        Ok(defs
            .iter()
            .find(|l| l.matches_name(&name))
            .map(|l| serde_json::json!({"id": l.id.0, "name": l.name, "dataSize": data_size_name(l.data)}).to_string()))
    });
    native!("layerDefaultValue", |h, c, id: f64| -> rquickjs::Result<i32> {
        let defs = h.worlds.iter().flatten().next().map(|w| w.layers.clone()).unwrap_or_else(meadow_core::layer::builtin_layers);
        Ok(match defs.iter().find(|l| l.id.0 == id as u16).map(|l| l.data) {
            Some(DataSize::Bit) => 1,
            Some(DataSize::Byte) => 128,
            _ => 8,
        })
    });
    native!("biomeId", |h, c, name: String| -> rquickjs::Result<i32> {
        let b = meadow_blocks::biomes();
        let key = name.to_lowercase().replace(' ', "_");
        Ok(b.by_id(&key).or_else(|| b.list.iter().find(|x| x.name.eq_ignore_ascii_case(&name))).map(|x| x.numeric as i32).unwrap_or(-1))
    });

    // Height maps
    native!("loadHeightMap", |h, c, path: String, channel: String| -> rquickjs::Result<f64> {
        let ch = match channel.as_str() {
            "red" => Channel::Red,
            "green" => Channel::Green,
            "blue" => Channel::Blue,
            _ => Channel::Grey,
        };
        let p = h.resolve(&path);
        let map = tri!(c, HeightMap::load(&p, ch).map_err(|e| format!("{e:#}")));
        h.heightmaps.push(map);
        Ok((h.heightmaps.len() - 1) as f64)
    });
    native!("hmWidth", |h, c, id: f64| -> rquickjs::Result<f64> { Ok(h.heightmaps.get(id as usize).map(|m| m.width as f64).unwrap_or(0.0)) });
    native!("hmHeight", |h, c, id: f64| -> rquickjs::Result<f64> { Ok(h.heightmaps.get(id as usize).map(|m| m.height as f64).unwrap_or(0.0)) });
    native!("hmGet", |h, c, id: f64, x: f64, y: f64| -> rquickjs::Result<f64> {
        let m = tri!(c, h.heightmaps.get(id as usize).ok_or("invalid height map"));
        if x < 0.0 || y < 0.0 || x >= m.width as f64 || y >= m.height as f64 {
            return Ok(0.0);
        }
        Ok(m.get(x as i32, y as i32) as f64)
    });
    native!("hmColour", |h, c, id: f64, x: f64, y: f64| -> rquickjs::Result<f64> {
        let m = tri!(c, h.heightmaps.get(id as usize).ok_or("invalid height map"));
        Ok(match m.rgba(x as i32, y as i32) {
            Some(p) => (((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32) as f64,
            None => {
                let v = m.get(x as i32, y as i32) / m.max_value * 255.0;
                let g = v.clamp(0.0, 255.0) as u32;
                ((0xff << 24) | (g << 16) | (g << 8) | g) as f64
            }
        })
    });

    // Worlds
    native!("loadWorld", |h, c, path: String| -> rquickjs::Result<f64> {
        let p = h.resolve(&path);
        if p.extension().is_some_and(|e| e == "world") {
            return Err(throw(&c, format!("{}: WorldPainter .world files can't be opened by Meadow yet; open or create the world in Meadow and run the script on it", p.display())));
        }
        let w = tri!(c, World::load(&p).map_err(|e| format!("{e:#}")));
        h.worlds.push(Some(w));
        Ok((h.worlds.len() - 1) as f64)
    });
    native!("saveWorld", |h, c, id: f64, path: String| -> rquickjs::Result<()> {
        let mut p = h.resolve(&path);
        if p.extension().is_some_and(|e| e == "world") {
            p.set_extension("meadow");
        }
        let w = tri!(c, h.world(id));
        tri!(c, w.save(&p).map_err(|e| format!("{e:#}")));
        (h.out)(&format!("Saved {}\n", p.display()));
        Ok(())
    });
    native!("createWorld", |h, c, spec: String| -> rquickjs::Result<f64> {
        let s: Json = tri!(c, serde_json::from_str(&spec).map_err(|e| e.to_string()));
        let hm = s["heightMap"].as_f64().ok_or("fromHeightMap() not called");
        let hm = tri!(c, hm);
        let map = tri!(c, h.heightmaps.get(hm as usize).cloned().ok_or("invalid height map"));
        let lower = s["lower"].as_i64().unwrap_or(-64) as i32;
        let upper = s["upper"].as_i64().unwrap_or(320) as i32;
        let limits = BuildLimits { min_y: lower, max_y: upper }.sanitized();
        let mut w = World::new(&NewWorld { width_tiles: 0, height_tiles: 0, limits, ..Default::default() });
        let mut st = ImportSettings::for_map(&map, limits.min_y, limits.max_y);
        st.image_low = s["imageLow"].as_f64().map(|v| v as f32).unwrap_or(0.0);
        st.image_high = s["imageHigh"].as_f64().map(|v| v as f32).unwrap_or(map.max_value);
        st.world_low = s["worldLow"].as_f64().map(|v| v as f32).unwrap_or(0.0f32.max(limits.min_y as f32));
        st.world_high = s["worldHigh"].as_f64().map(|v| v as f32).unwrap_or(if map.max_value <= 255.0 { 255.0 } else { (limits.max_y - 1) as f32 });
        st.water_level = s["waterLevel"].as_i64().unwrap_or(62) as i32;
        st.scale_percent = s["scale"].as_f64().unwrap_or(100.0) as f32;
        st.offset = (s["offsetX"].as_i64().unwrap_or(0) as i32, s["offsetY"].as_i64().unwrap_or(0) as i32);
        st.resample = Resample::Bicubic;
        let wl = st.water_level;
        let d = w.surface_mut();
        d.default_water_level = wl;
        import_into(d, &map, &st);
        // Beaches around the water line, as WorldPainter's default import does.
        for (tx, ty) in d.tile_coords() {
            let t = d.tile_mut(tx, ty).unwrap();
            for i in 0..t.height.len() {
                let hh = (t.height[i] + 0.5).floor() as i32;
                if hh <= wl + 1 && hh >= wl - 4 {
                    t.terrain[i] = ids::BEACHES;
                }
            }
        }
        d.clear_history();
        if let Some(f) = s["format"].as_str()
            && let Some(v) = version_from_format(f)
        {
            w.export.version = v;
        }
        h.worlds.push(Some(w));
        Ok((h.worlds.len() - 1) as f64)
    });
    native!("exportWorld", |h, c, spec: String| -> rquickjs::Result<String> {
        let s: Json = tri!(c, serde_json::from_str(&spec).map_err(|e| e.to_string()));
        let id = s["world"].as_f64().unwrap_or(0.0);
        let dir = h.resolve(s["dir"].as_str().unwrap_or("."));
        let w = tri!(c, h.world(id));
        let version = s["format"].as_str().and_then(version_from_format).unwrap_or(w.export.version);
        let name = sanitize(&w.name);
        let opts = meadow_export::ExportOptions {
            dir: dir.join(name),
            version,
            dimensions: w.dimensions.iter().map(|d| d.kind).collect(),
            compression: 6,
            overwrite: true,
        };
        let stats = tri!(c, meadow_export::export_world(w, &opts, &meadow_export::Progress::default()).map_err(|e| format!("{e:#}")));
        let mut msg = format!("Exported {} chunks to {} in {:.1}s", stats.chunks, opts.dir.display(), stats.elapsed.as_secs_f32());
        for wn in stats.warnings.iter().take(10) {
            msg += &format!("\n  warning: {wn}");
        }
        Ok(msg)
    });
    native!("worldName", |h, c, id: f64| -> rquickjs::Result<String> { Ok(tri!(c, h.world(id)).name.clone()) });
    native!("setWorldName", |h, c, id: f64, n: String| -> rquickjs::Result<()> {
        tri!(c, h.world(id)).name = n;
        Ok(())
    });
    native!("seed", |h, c, id: f64| -> rquickjs::Result<f64> { Ok(tri!(c, h.world(id)).seed as f64) });
    native!("spawn", |h, c, id: f64| -> rquickjs::Result<String> {
        let s = tri!(c, h.world(id)).spawn;
        Ok(format!("[{},{}]", s.0, s.1))
    });
    native!("setSpawn", |h, c, id: f64, x: f64, y: f64| -> rquickjs::Result<()> {
        tri!(c, h.world(id)).spawn = (x as i32, y as i32);
        Ok(())
    });
    native!("dimExists", |h, c, id: f64, d: f64| -> rquickjs::Result<bool> { Ok(h.dim(id, d).is_ok()) });

    // Dimension access (hot paths: keep them lean)
    native!("getHeight", |h, c, w: f64, d: f64, x: f64, y: f64| -> rquickjs::Result<f64> {
        let dim = tri!(c, h.dim(w, d));
        Ok(dim.height(x as i32, y as i32).map(|v| v as f64).unwrap_or(dim.limits.min_y as f64))
    });
    native!("getIntHeight", |h, c, w: f64, d: f64, x: f64, y: f64| -> rquickjs::Result<i32> {
        let dim = tri!(c, h.dim(w, d));
        Ok(dim.int_height(x as i32, y as i32).unwrap_or(dim.limits.min_y))
    });
    native!("setHeight", |h, c, w: f64, d: f64, x: f64, y: f64, v: f64| -> rquickjs::Result<()> {
        tri!(c, h.dim(w, d)).set_height(x as i32, y as i32, v as f32);
        Ok(())
    });
    native!("getTerrain", |h, c, w: f64, d: f64, x: f64, y: f64| -> rquickjs::Result<i32> {
        Ok(tri!(c, h.dim(w, d)).terrain(x as i32, y as i32).map(|t| t as i32).unwrap_or(-1))
    });
    native!("setTerrain", |h, c, w: f64, d: f64, x: f64, y: f64, t: f64| -> rquickjs::Result<()> {
        tri!(c, h.dim(w, d)).set_terrain(x as i32, y as i32, t as u8);
        Ok(())
    });
    native!("getWater", |h, c, w: f64, d: f64, x: f64, y: f64| -> rquickjs::Result<i32> {
        let dim = tri!(c, h.dim(w, d));
        Ok(dim.water(x as i32, y as i32).unwrap_or(dim.limits.min_y))
    });
    native!("setWater", |h, c, w: f64, d: f64, x: f64, y: f64, v: f64| -> rquickjs::Result<()> {
        tri!(c, h.dim(w, d)).set_water(x as i32, y as i32, v as i32);
        Ok(())
    });
    native!("getLayer", |h, c, w: f64, d: f64, l: f64, x: f64, y: f64| -> rquickjs::Result<i32> {
        let id = LayerId(l as u16);
        let def = tri!(c, h.world(w)).layer_default(id);
        Ok(tri!(c, h.dim(w, d)).layer_value(id, def, x as i32, y as i32) as i32)
    });
    native!("setLayer", |h, c, w: f64, d: f64, l: f64, x: f64, y: f64, v: f64| -> rquickjs::Result<()> {
        let id = LayerId(l as u16);
        let world = tri!(c, h.world(w));
        let (bits, def, max) = world.layer(id).map(|l| (l.is_bits(), l.default, l.data.max())).unwrap_or((false, 0, 15));
        tri!(c, h.dim(w, d)).set_layer_value(id, bits, def, x as i32, y as i32, (v.max(0.0) as u32).min(max as u32) as u8);
        Ok(())
    });
    native!("clearLayer", |h, c, w: f64, d: f64, l: f64| -> rquickjs::Result<()> {
        let id = LayerId(l as u16);
        let dim = tri!(c, h.dim(w, d));
        for (tx, ty) in dim.tile_coords() {
            if let Some(t) = dim.tile_mut(tx, ty) {
                t.layers.retain(|(x, _)| *x != id);
            }
        }
        Ok(())
    });
    native!("getSlope", |h, c, w: f64, d: f64, x: f64, y: f64| -> rquickjs::Result<f64> {
        Ok(tri!(c, h.dim(w, d)).slope(x as i32, y as i32).to_radians().tan() as f64)
    });
    native!("tilePresent", |h, c, w: f64, d: f64, tx: f64, ty: f64| -> rquickjs::Result<bool> {
        Ok(tri!(c, h.dim(w, d)).tile(tx as i32, ty as i32).is_some())
    });
    native!("tileCount", |h, c, w: f64, d: f64| -> rquickjs::Result<f64> { Ok(tri!(c, h.dim(w, d)).tile_count() as f64) });
    native!("tileCoords", |h, c, w: f64, d: f64| -> rquickjs::Result<String> {
        let v: Vec<[i32; 2]> = tri!(c, h.dim(w, d)).tile_coords().into_iter().map(|(x, y)| [x, y]).collect();
        Ok(serde_json::to_string(&v).unwrap())
    });
    native!("bounds", |h, c, w: f64, d: f64| -> rquickjs::Result<String> {
        let b = tri!(c, h.dim(w, d)).tile_bounds().unwrap_or((0, 0, -1, -1));
        Ok(format!("[{},{},{},{}]", b.0, b.1, b.2, b.3))
    });
    native!("minHeight", |h, c, w: f64, d: f64| -> rquickjs::Result<i32> { Ok(tri!(c, h.dim(w, d)).limits.min_y) });
    native!("maxHeight", |h, c, w: f64, d: f64| -> rquickjs::Result<i32> { Ok(tri!(c, h.dim(w, d)).limits.max_y) });

    native!("createFilter", |h, c, spec: String| -> rquickjs::Result<f64> {
        let s: Json = tri!(c, serde_json::from_str(&spec).map_err(|e| e.to_string()));
        let mask = build_filter(&s);
        h.filters.push(mask);
        Ok((h.filters.len() - 1) as f64)
    });
    native!("mapping", |h, c, spec: String| -> rquickjs::Result<()> {
        let s: Json = tri!(c, serde_json::from_str(&spec).map_err(|e| e.to_string()));
        tri!(c, mapping_op(h, &s));
        Ok(())
    });

    ctx.globals().set("__m", m)?;
    Ok(())
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
    if s.trim().is_empty() { "world".into() } else { s }
}

/// Map WorldPainter platform IDs (e.g. "org.pepsoft.anvil.1.20.5") or plain versions to a target.
pub fn version_from_format(f: &str) -> Option<McVersion> {
    let digits: String = f.chars().skip_while(|c| !c.is_ascii_digit()).collect();
    let parts: Vec<u32> = digits.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.is_empty() {
        return None;
    }
    let key = |v: &McVersion| -> Vec<u32> { v.name().split('.').filter_map(|p| p.parse().ok()).collect() };
    McVersion::ALL.into_iter().find(|v| key(v) >= parts).or(Some(McVersion::LATEST))
}

fn build_filter(s: &Json) -> CustomMask {
    let mut and: Vec<MaskNode> = Vec::new();
    let feather = if s["feather"].as_bool() == Some(true) { 8.0 } else { 0.0 };
    let above = s["above"].as_f64();
    let below = s["below"].as_f64();
    if above.is_some() || below.is_some() {
        and.push(MaskNode::Height {
            min: above.map(|v| v as f32).unwrap_or(-10000.0),
            max: below.map(|v| v as f32).unwrap_or(10000.0),
            feather,
        });
    }
    if s["aboveDeg"].is_number() || s["belowDeg"].is_number() {
        and.push(MaskNode::Slope {
            min: s["aboveDeg"].as_f64().unwrap_or(-1.0) as f32,
            max: s["belowDeg"].as_f64().unwrap_or(91.0) as f32,
            feather: 0.0,
        });
    }
    let cond = |c: &Json| -> MaskNode {
        if let Some(t) = c["terrain"].as_u64() {
            MaskNode::Terrain(vec![t as u8])
        } else if let Some(l) = c["layer"].as_u64() {
            let id = LayerId(l as u16);
            match (c["cmp"].as_str(), c["value"].as_u64()) {
                (Some("eq"), Some(v)) => MaskNode::And(vec![
                    MaskNode::Layer { layer: id, min: v as u8 },
                    MaskNode::Not(Box::new(MaskNode::Layer { layer: id, min: v as u8 + 1 })),
                ]),
                (Some("ge"), Some(v)) => MaskNode::Layer { layer: id, min: v as u8 },
                (Some("le"), Some(v)) => MaskNode::Not(Box::new(MaskNode::Layer { layer: id, min: v as u8 + 1 })),
                _ => MaskNode::Layer { layer: id, min: 1 },
            }
        } else if let Some(b) = c["biome"].as_u64().or(c["autoBiome"].as_u64()) {
            MaskNode::Biome(vec![b as u8])
        } else if c["autoBiomes"].as_bool() == Some(true) {
            MaskNode::Biome(vec![meadow_blocks::BIOME_AUTO])
        } else if let Some(w) = c["water"].as_bool() {
            MaskNode::Water(w)
        } else {
            MaskNode::All
        }
    };
    if let Some(only) = s["only"].as_array()
        && !only.is_empty()
    {
        and.push(MaskNode::Or(only.iter().map(cond).collect()));
    }
    if let Some(ex) = s["except"].as_array()
        && !ex.is_empty()
    {
        and.push(MaskNode::Not(Box::new(MaskNode::Or(ex.iter().map(cond).collect()))));
    }
    CustomMask { name: "script filter".into(), node: if and.is_empty() { MaskNode::All } else { MaskNode::And(and) }, color: [255, 255, 0], invert: false }
}

/// WorldPainter's MappingOp.go(), natively.
fn mapping_op(h: &mut Host, s: &Json) -> Result<(), String> {
    let world_id = s["world"].as_f64().ok_or("world not specified")?;
    let dim_idx = s["dim"].as_f64().unwrap_or(0.0);
    let hm = s["heightMap"].as_f64().map(|v| v as usize);
    let layer = s["layer"].as_u64().map(|v| LayerId(v as u16));
    let mode = s["mode"].as_str().unwrap_or("set").to_owned();
    let terrain_mode = mode == "terrain";
    let filter = s["filter"].as_f64().and_then(|f| h.filters.get(f as usize).cloned());
    let scale = s["scale"].as_f64().unwrap_or(100.0) as f32 / 100.0;
    let (ox, oy) = (s["offsetX"].as_i64().unwrap_or(0) as i32, s["offsetY"].as_i64().unwrap_or(0) as i32);
    if !terrain_mode && layer.is_none() {
        return Err("layer not specified".into());
    }
    let (bits, layer_default, max) = match layer {
        Some(l) => {
            let w = h.world(world_id)?;
            let d = w.layer(l).ok_or("unknown layer")?;
            (d.is_bits(), d.default, d.data.max() as i32)
        }
        None => (false, 0, 255),
    };
    // Build the grey-scale lookup table and the colour map.
    let mut table = vec![-1i32; 65536];
    let mut grey = false;
    for m in s["mapping"].as_array().into_iter().flatten() {
        let v: Vec<i64> = m.as_array().map(|a| a.iter().filter_map(Json::as_i64).collect()).unwrap_or_default();
        if v.len() != 4 {
            continue;
        }
        let (lo, hi, tlo, thi) = (v[0].clamp(0, 65535) as usize, v[1].clamp(0, 65535) as usize, v[2], v[3]);
        grey = true;
        if lo == hi || tlo == thi {
            for e in &mut table[lo..=hi.max(lo)] {
                *e = tlo as i32;
            }
        } else {
            let k = (thi - tlo) as f32 / (hi - lo) as f32;
            for i in lo..=hi {
                table[i] = tlo as i32 + ((i - lo) as f32 * k).round() as i32;
            }
        }
    }
    let colours: Vec<(u32, i32)> = s["colours"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let a = e.as_array()?;
            Some((a.first()?.as_u64()? as u32, a.get(1)?.as_i64()? as i32))
        })
        .collect();
    let map = match hm {
        Some(i) => Some(h.heightmaps.get(i).cloned().ok_or("invalid height map")?),
        None => None,
    };
    // Default mapping when only a value map and a layer were given: non-zero applies the layer.
    if map.is_some() && !grey && colours.is_empty() {
        if terrain_mode {
            return Err("mapping not specified".into());
        }
        let m = map.as_ref().unwrap();
        let mv = m.max_value.max(1.0);
        for (i, e) in table.iter_mut().enumerate().skip(1) {
            *e = if bits { 1 } else { ((i as f32 / mv) * max as f32).round().clamp(1.0, max as f32) as i32 };
        }
        grey = true;
    }
    for (_, v) in &colours {
        if *v < 0 || (!terrain_mode && *v > max) {
            return Err(format!("invalid destination value {v}"));
        }
    }
    let direct_value = if terrain_mode { s["terrain"].as_i64().unwrap_or(0) as i32 } else { s["layerValue"].as_i64().unwrap_or(max.min(8) as i64) as i32 };
    let smooth = grey && !terrain_mode && layer != Some(LayerId::BIOME) && layer != Some(LayerId::ANNOTATIONS);

    let dim = h.dim(world_id, dim_idx)?;
    let Some((bx0, by0, bx1, by1)) = dim.block_bounds() else { return Ok(()) };
    let (mut x0, mut y0, mut x1, mut y1) = (bx0, by0, bx1, by1);
    if let Some(m) = &map {
        x0 = x0.max(ox);
        y0 = y0.max(oy);
        x1 = x1.min(ox + (m.width as f32 * scale).round() as i32);
        y1 = y1.min(oy + (m.height as f32 * scale).round() as i32);
    }
    let snapshot = dim.snapshot();
    for y in y0..y1 {
        for x in x0..x1 {
            if snapshot.height(x, y).is_none() {
                continue;
            }
            let mut value = if let Some(m) = &map {
                let (sx, sy) = (((x - ox) as f32 / scale) as i32, ((y - oy) as f32 / scale) as i32);
                if !colours.is_empty() {
                    let c = match m.rgba(sx, sy) {
                        Some(p) => ((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32,
                        None => {
                            let g = (m.get(sx, sy) / m.max_value * 255.0).clamp(0.0, 255.0) as u32;
                            0xff00_0000 | (g << 16) | (g << 8) | g
                        }
                    };
                    match colours.iter().find(|(k, _)| *k == c) {
                        Some((_, v)) => *v,
                        None => continue,
                    }
                } else {
                    let vin = m.get(sx, sy).round() as i64;
                    if !(0..=65535).contains(&vin) {
                        continue;
                    }
                    let v = table[vin as usize];
                    if v == -1 {
                        continue;
                    }
                    v
                }
            } else {
                direct_value
            };
            if let Some(f) = &filter {
                let fv = f.eval(&snapshot, x, y);
                if fv <= 0.0 {
                    continue;
                } else if smooth && fv < 1.0 {
                    value = (fv * value as f32).round() as i32;
                }
            }
            if terrain_mode {
                dim.set_terrain(x, y, value.clamp(0, 255) as u8);
                continue;
            }
            let l = layer.unwrap();
            let cur = snapshot.layer_value(l, layer_default, x, y) as i32;
            let v = value.clamp(0, max) as u8;
            match mode.as_str() {
                "setWhenHigher" => {
                    if bits {
                        if v != 0 {
                            dim.set_layer_value(l, true, layer_default, x, y, 1);
                        }
                    } else if cur < value {
                        dim.set_layer_value(l, false, layer_default, x, y, v);
                    }
                }
                "setWhenLower" => {
                    if bits {
                        if v == 0 {
                            dim.set_layer_value(l, true, layer_default, x, y, 0);
                        }
                    } else if cur > value {
                        dim.set_layer_value(l, false, layer_default, x, y, v);
                    }
                }
                _ => dim.set_layer_value(l, bits, layer_default, x, y, if bits { (v != 0) as u8 } else { v }),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
