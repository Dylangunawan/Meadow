//! The Meadow editor application.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText};
use meadow_core::layer::{LayerDef, LayerId, LayerKind};
use meadow_core::ops::{self, Brush, Paint, PaintMode, Stroke, Tool};
use meadow_core::world::NewWorld;
use meadow_core::{DimensionKind, World};
use meadow_render::Renderer;

use crate::dialogs::Dialogs;
use crate::jobs::{ExportJob, JarJob, ScriptJob};
use crate::map2d::{Map2D, MapAction, MapStyle, MapUi};
use crate::settings::{Dock, PanelKind, Settings};
use crate::view3d::{ObjectPreview, View3D};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Map,
    View3D,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolChoice {
    /// No tool (Esc): the map only navigates.
    None,
    // WorldPainter's tools
    SprayPaint,
    Pencil,
    Fill,
    Flood,
    FloodLava,
    Sponge,
    Height,
    Flatten,
    Smooth,
    Spawn,
    // Meadow extras
    SetHeight,
    Roughen,
    Terrace,
    WaterLevel,
    Mask,
    Tiles,
}

/// One entry of the tool palette: tool, label, tooltip, Alt+key shortcut.
pub struct ToolInfo {
    pub tool: ToolChoice,
    pub name: &'static str,
    pub tip: &'static str,
    pub key: Option<Key>,
}

const fn ti(tool: ToolChoice, name: &'static str, tip: &'static str, key: Option<Key>) -> ToolInfo {
    ToolInfo { tool, name, tip, key }
}

/// What Spray Paint, Pencil and Fill apply: the last terrain, layer or biome picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintKind {
    Terrain,
    Layer,
    Biome,
}

impl ToolChoice {
    /// WorldPainter's tool grid (4 columns), in its order.
    pub const WORLDPAINTER: [ToolInfo; 10] = [
        ti(ToolChoice::SprayPaint, "Spray Paint", "Spray paint the selected terrain, layer or biome (Alt+R)", Some(Key::R)),
        ti(ToolChoice::Pencil, "Pencil", "Draw with the selected terrain, layer or biome, hard-edged (Alt+P)", Some(Key::P)),
        ti(ToolChoice::Fill, "Fill", "Flood fill an area with the selected paint (Alt+L)", Some(Key::L)),
        ti(ToolChoice::Flood, "Flood", "Click to raise the water by one block, right-click to lower (Alt+F)", Some(Key::F)),
        ti(ToolChoice::FloodLava, "Flood Lava", "Click to raise lava by one block, or turn water into lava", None),
        ti(ToolChoice::Sponge, "Sponge", "Dry up water and lava; right-click resets the water level", None),
        ti(ToolChoice::Height, "Height", "Raise the terrain; right-click lowers it (Alt+H)", Some(Key::H)),
        ti(ToolChoice::Flatten, "Flatten", "Flatten to the height where the stroke started (Alt+A)", Some(Key::A)),
        ti(ToolChoice::Smooth, "Smooth", "Smooth the terrain (Alt+S)", Some(Key::S)),
        ti(ToolChoice::Spawn, "Spawn", "Click to set the spawn point", None),
    ];
    pub const EXTRAS: [ToolInfo; 6] = [
        ti(ToolChoice::SetHeight, "Set height", "Move the terrain towards a fixed height", None),
        ti(ToolChoice::Roughen, "Roughen", "Add noise to the terrain", None),
        ti(ToolChoice::Terrace, "Terrace", "Shape the terrain into terraces", None),
        ti(ToolChoice::WaterLevel, "Water level", "Set the water level; right-click removes water", None),
        ti(ToolChoice::Mask, "Paint mask", "Paint the selected painted mask; right-click erases", None),
        ti(ToolChoice::Tiles, "Tiles", "Add tiles to the world; right-click removes them (Ctrl+T)", None),
    ];
    pub fn info(self) -> Option<&'static ToolInfo> {
        Self::WORLDPAINTER.iter().chain(Self::EXTRAS.iter()).find(|t| t.tool == self)
    }
    pub fn name(self) -> &'static str {
        self.info().map(|t| t.name).unwrap_or("No tool")
    }
    pub fn is_click(self) -> bool {
        matches!(self, ToolChoice::Flood | ToolChoice::FloodLava | ToolChoice::Fill | ToolChoice::Spawn)
    }
    pub fn uses_brush(self) -> bool {
        !matches!(self, ToolChoice::None | ToolChoice::Flood | ToolChoice::FloodLava | ToolChoice::Fill | ToolChoice::Spawn)
    }
    pub fn uses_paint(self) -> bool {
        matches!(self, ToolChoice::SprayPaint | ToolChoice::Pencil | ToolChoice::Fill)
    }
}

pub struct Gpu {
    pub rs: egui_wgpu::RenderState,
    pub r: Renderer,
}

pub struct MeadowApp {
    pub settings: Settings,
    pub settings_applied: bool,
    pub world: World,
    pub dim: DimensionKind,
    pub gpu: Option<Gpu>,
    pub view_mode: ViewMode,
    pub map: Map2D,
    pub view3d: Option<View3D>,
    pub preview: Option<ObjectPreview>,
    pub tool: ToolChoice,
    pub brush: Brush,
    pub paint_kind: PaintKind,
    pub terrain: u8,
    pub layer: Option<LayerId>,
    pub biome: u8,
    pub height_level: f32,
    pub water_level: i32,
    pub mask_filter: Option<usize>,
    pub mask_overlay: bool,
    pub paint_mask_layer: Option<LayerId>,
    /// Layers whose "show" box is unticked.
    pub hidden_layers: Vec<LayerId>,
    pub stroke: Stroke,
    /// The pointer was over the 3D view last frame (keys go to the 3D view).
    pub hover_3d: bool,
    last_shortcut_frame: u64,
    pub defs_gen: u64,
    pub dialogs: Dialogs,
    pub export_job: Option<ExportJob>,
    pub script_job: Option<ScriptJob>,
    pub jar_job: Option<JarJob>,
    pub status: String,
    pub toasts: Vec<(String, Instant)>,
    pub last_autosave: Instant,
    pub frame_map: bool,
    pub last_title: String,
}

impl MeadowApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> MeadowApp {
        let settings = Settings::load();
        let imported = crate::jobs::load_imported_blocks();
        let gpu = cc.wgpu_render_state.clone().map(|rs| {
            let r = Renderer::new(&rs.adapter, rs.device.clone(), rs.queue.clone());
            Gpu { rs, r }
        });
        let view3d = gpu.as_ref().map(|g| View3D::new(&g.r));
        let preview = gpu.as_ref().map(|g| ObjectPreview::new(&g.r));
        let world = World::new(&NewWorld { hills: 24.0, ..Default::default() });
        let mut app = MeadowApp {
            settings,
            settings_applied: false,
            world,
            dim: DimensionKind::Overworld,
            gpu,
            view_mode: ViewMode::Map,
            map: Map2D::new(),
            view3d,
            preview,
            tool: ToolChoice::Height,
            brush: Brush::default(),
            paint_kind: PaintKind::Terrain,
            terrain: 0,
            layer: Some(LayerId::DECIDUOUS),
            biome: 1,
            height_level: 64.0,
            water_level: 62,
            mask_filter: None,
            mask_overlay: false,
            paint_mask_layer: None,
            hidden_layers: Vec::new(),
            stroke: Stroke::default(),
            hover_3d: false,
            last_shortcut_frame: u64::MAX,
            defs_gen: 1,
            dialogs: Dialogs::default(),
            export_job: None,
            script_job: None,
            jar_job: None,
            status: String::new(),
            toasts: Vec::new(),
            last_autosave: Instant::now(),
            frame_map: true,
            last_title: String::new(),
        };
        if imported > 0 {
            app.toast(format!("Loaded {imported} imported blocks and colours"));
        }
        // Like WorldPainter's colour scheme: use the real block colours from the user's own
        // Minecraft installation (checked once; View > Change colour scheme redoes it).
        if !app.settings.colors_auto_checked {
            app.settings.colors_auto_checked = true;
            app.settings.save();
            if let Some(jar) = meadow_blocks::modjar::find_minecraft_jar() {
                app.toast(format!("Reading block colours from {}", jar.display()));
                app.jar_job = Some(JarJob::start(jar, cc.egui_ctx.clone()));
            }
        }
        // Open a world passed on the command line.
        if let Some(p) = std::env::args().nth(1).map(PathBuf::from)
            && p.extension().is_some_and(|e| e == "meadow")
        {
            app.open_world(&p);
        }
        app
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        log::info!("{m}");
        self.toasts.push((m, Instant::now()));
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.dialogs.error = Some(msg.into());
    }

    pub fn dim_mut(&mut self) -> &mut meadow_core::Dimension {
        let kind = self.dim;
        if self.world.dimension(kind).is_none() {
            self.dim = DimensionKind::Overworld;
        }
        let kind = self.dim;
        self.world.dimension_mut(kind).unwrap()
    }

    pub fn dim(&self) -> &meadow_core::Dimension {
        self.world.dimension(self.dim).unwrap_or_else(|| self.world.surface())
    }

    /// World definitions (layers, terrains, masks) changed: refresh caches and previews.
    pub fn defs_changed(&mut self) {
        self.defs_gen += 1;
        self.world.dirty = true;
        if let Some(p) = &mut self.preview {
            p.dirty = true;
        }
    }

    pub fn set_world(&mut self, w: World) {
        self.world = w;
        self.dim = DimensionKind::Overworld;
        self.water_level = self.world.surface().default_water_level;
        self.map.clear_cache();
        self.frame_map = true;
        if let Some(v) = &mut self.view3d {
            v.refocus();
        }
        self.defs_changed();
        self.world.dirty = false;
        self.mask_filter = None;
    }

    pub fn open_world(&mut self, path: &std::path::Path) {
        match World::load(path) {
            Ok(w) => {
                self.set_world(w);
                self.settings.push_recent(path.to_owned());
                self.settings.save();
                self.toast(format!("Opened {}", path.display()));
            }
            Err(e) => self.error(format!("Could not open {}:\n{e:#}", path.display())),
        }
    }

    pub fn save_world(&mut self, path: Option<PathBuf>) {
        let path = match path.or_else(|| self.world.path.clone()) {
            Some(p) => p,
            None => match rfd::FileDialog::new().add_filter("Meadow world", &["meadow"]).set_file_name(format!("{}.meadow", self.world.name)).save_file() {
                Some(p) => p,
                None => return,
            },
        };
        let path = if path.extension().is_none() { path.with_extension("meadow") } else { path };
        let t = Instant::now();
        match self.world.save(&path) {
            Ok(()) => {
                self.settings.push_recent(path.clone());
                self.settings.save();
                self.toast(format!("Saved {} in {:.2}s", path.display(), t.elapsed().as_secs_f32()));
            }
            Err(e) => self.error(format!("Could not save:\n{e:#}")),
        }
    }

    /// The paint Spray Paint, Pencil and Fill apply.
    pub fn current_paint(&self) -> Option<Paint> {
        Some(match self.paint_kind {
            PaintKind::Terrain => Paint::Terrain(self.terrain),
            PaintKind::Biome => Paint::Discrete { layer: LayerId::BIOME, value: self.biome, default: meadow_blocks::BIOME_AUTO },
            PaintKind::Layer => {
                let d: LayerDef = self.world.layer(self.layer?)?.clone();
                match d.kind {
                    LayerKind::Annotations => Paint::Discrete { layer: d.id, value: 1, default: 0 },
                    _ => Paint::Layer { layer: d.id, bits: d.is_bits(), default: d.default },
                }
            }
        })
    }

    pub fn paint_name(&self) -> String {
        match self.paint_kind {
            PaintKind::Terrain => self.world.terrain(self.terrain).name.clone(),
            PaintKind::Biome => meadow_blocks::biomes().by_numeric(self.biome).map(|b| b.name.clone()).unwrap_or_default(),
            PaintKind::Layer => self.layer.and_then(|l| self.world.layer(l)).map(|l| l.name.clone()).unwrap_or_else(|| "none".into()),
        }
    }

    /// Select a paint from the Terrain/Layers/Biomes panels; switch to Spray Paint if the
    /// current tool doesn't paint.
    pub fn select_paint(&mut self, kind: PaintKind) {
        self.paint_kind = kind;
        if !self.tool.uses_paint() {
            self.tool = ToolChoice::SprayPaint;
        }
    }

    fn core_tool(&self, inverse: bool) -> Option<Tool> {
        let default_water = self.dim().default_water_level;
        Some(match self.tool {
            ToolChoice::None | ToolChoice::Fill | ToolChoice::Flood | ToolChoice::FloodLava | ToolChoice::Spawn | ToolChoice::Tiles => return None,
            ToolChoice::SprayPaint => Tool::Paint { paint: self.current_paint()?, mode: PaintMode::Spray, remove: inverse },
            ToolChoice::Pencil => Tool::Paint { paint: self.current_paint()?, mode: PaintMode::Pencil, remove: inverse },
            ToolChoice::Height => {
                if inverse {
                    Tool::Lower
                } else {
                    Tool::Raise
                }
            }
            ToolChoice::Flatten => Tool::Flatten,
            ToolChoice::Smooth => Tool::Smooth,
            ToolChoice::Sponge => {
                if inverse {
                    Tool::ResetWater(default_water)
                } else {
                    Tool::Sponge
                }
            }
            ToolChoice::SetHeight => Tool::SetHeight(self.height_level),
            ToolChoice::Roughen => Tool::Roughen,
            ToolChoice::Terrace => Tool::Terrace(8.0),
            ToolChoice::WaterLevel => {
                if inverse {
                    Tool::Sponge
                } else {
                    Tool::SetWaterLevel(self.water_level)
                }
            }
            ToolChoice::Mask => {
                let l = self.paint_mask_layer?;
                if inverse { Tool::EraseMask(l) } else { Tool::PaintMask(l) }
            }
        })
    }

    fn paint(&mut self, from: (f32, f32), to: (f32, f32), dt: f32, first: bool, inverse: bool) {
        let tool = self.core_tool(inverse);
        let tiles_tool = self.tool == ToolChoice::Tiles;
        if tool.is_none() && !tiles_tool {
            if self.tool.uses_paint() {
                self.status = "Pick a terrain, layer or biome to paint with".into();
            } else if self.tool == ToolChoice::Mask {
                self.status = "Create or select a painted mask in the Custom masks panel".into();
            }
            return;
        }
        let kind = self.dim;
        let brush = self.brush;
        let mask_idx = self.mask_filter;
        let base = self.height_level;
        let World { dimensions, masks, .. } = &mut self.world;
        let Some(dim) = dimensions.iter_mut().find(|d| d.kind == kind) else { return };
        if first {
            let name = tool.as_ref().map(|t| t.name()).unwrap_or("Tiles");
            dim.begin_op(name);
            self.stroke = Stroke::default();
        }
        let mask = mask_idx.and_then(|i| masks.get(i));
        let dist = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
        let spacing = (brush.radius * 0.3).max(1.0);
        let steps = ((dist / spacing).ceil() as usize).clamp(1, 256);
        let continuous = tool.as_ref().is_some_and(|t| t.is_continuous());
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            let (x, y) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            if tiles_tool {
                let r = brush.radius.ceil() as i32;
                let (x0, y0, x1, y1) = ((x as i32 - r) >> 7, (y as i32 - r) >> 7, (x as i32 + r) >> 7, (y as i32 + r) >> 7);
                for ty in y0..=y1 {
                    for tx in x0..=x1 {
                        if inverse {
                            dim.remove_tile(tx, ty);
                        } else {
                            dim.ensure_tile(tx, ty, base);
                        }
                    }
                }
                continue;
            }
            let step_dt = if continuous { dt / steps as f32 } else { 1.0 };
            if !continuous && k < steps && first {
                continue;
            }
            ops::apply(dim, tool.as_ref().unwrap(), &brush, x, y, step_dt, mask, &mut self.stroke);
        }
        self.world.dirty = true;
    }

    fn end_paint(&mut self) {
        let kind = self.dim;
        if let Some(d) = self.world.dimension_mut(kind) {
            d.end_op();
        }
    }

    fn click(&mut self, x: i32, y: i32, inverse: bool) {
        match self.tool {
            ToolChoice::Flood | ToolChoice::FloodLava => {
                let lava = self.tool == ToolChoice::FloodLava;
                let d = self.dim_mut();
                d.begin_op(if lava { "Flood with lava" } else { "Flood" });
                let n = ops::flood_step(d, x, y, inverse, lava, 16_000_000);
                d.end_op();
                let level = self.dim().water(x, y).unwrap_or(0);
                self.status = if n == 0 {
                    "Nothing to flood here".into()
                } else {
                    format!("{} {n} columns; {} level {level}", if inverse { "Lowered" } else { "Flooded" }, if lava { "lava" } else { "water" })
                };
                self.world.dirty = true;
            }
            ToolChoice::Fill => {
                let Some(paint) = self.current_paint() else {
                    self.status = "Pick a terrain, layer or biome to fill with".into();
                    return;
                };
                let d = self.dim_mut();
                d.begin_op("Fill");
                let n = ops::fill_paint(d, x, y, &paint, inverse, 16_000_000);
                d.end_op();
                self.status = format!("Filled {n} columns");
                self.world.dirty = true;
            }
            ToolChoice::Spawn => {
                self.world.spawn = (x, y);
                self.world.dirty = true;
                self.status = format!("Spawn set to {x}, {y}");
            }
            _ => {}
        }
    }

    pub fn undo(&mut self) {
        if let Some(n) = self.dim_mut().undo() {
            self.status = format!("Undid {n}");
        }
    }
    pub fn redo(&mut self) {
        if let Some(n) = self.dim_mut().redo() {
            self.status = format!("Redid {n}");
        }
    }

    /// WorldPainter's keyboard shortcuts (plus a few Meadow extras that don't clash).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        // egui may run more than one pass per frame; handle keys once.
        let frame = ctx.cumulative_frame_nr();
        if frame == self.last_shortcut_frame {
            return;
        }
        self.last_shortcut_frame = frame;
        let typing = ctx.egui_wants_keyboard_input();
        let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
        let fire = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let ppp = ctx.pixels_per_point();
        // Shift variants first: egui's matching ignores Shift when it isn't asked for.
        if fire(sc(cmd_shift, Key::S)) {
            if let Some(p) = rfd::FileDialog::new().add_filter("Meadow world", &["meadow"]).set_file_name(format!("{}.meadow", self.world.name)).save_file() {
                self.save_world(Some(p));
            }
        } else if fire(sc(cmd, Key::S)) {
            self.save_world(None);
        }
        if fire(sc(cmd_shift, Key::Z)) || fire(sc(cmd, Key::Y)) {
            self.redo();
        } else if fire(sc(cmd, Key::Z)) {
            self.undo();
        }
        if fire(sc(cmd, Key::O)) {
            self.dialogs.open_file = true;
        }
        if fire(sc(cmd, Key::N)) {
            self.dialogs.new_world.open = true;
        }
        if fire(sc(cmd, Key::E)) {
            self.dialogs.export.open = true;
        }
        if fire(sc(cmd, Key::G)) {
            self.dialogs.fill.open = true;
        }
        if fire(sc(cmd, Key::T)) {
            self.tool = ToolChoice::Tiles;
            self.status = "Add tiles by painting; right-click removes them".into();
        }
        if fire(sc(cmd, Key::P)) {
            self.dialogs.world_settings.open_for(&self.world);
        }
        if fire(sc(cmd, Key::V)) {
            self.dialogs.configure_view = true;
        }
        for (key, kind) in [(Key::U, DimensionKind::Overworld), (Key::H, DimensionKind::Nether), (Key::D, DimensionKind::End)] {
            if fire(sc(cmd, key)) {
                if self.world.dimension(kind).is_some() {
                    self.dim = kind;
                    self.frame_map = true;
                } else {
                    self.status = format!("This world has no {}; add it from the Dimension menu", kind.name());
                }
            }
        }
        if fire(sc(cmd, Key::Num3)) {
            self.view_mode = if self.view_mode == ViewMode::Map { ViewMode::View3D } else { ViewMode::Map };
        }
        if fire(sc(cmd, Key::Equals)) || fire(sc(cmd, Key::Plus)) {
            self.map.zoom_keyboard(1, ppp);
        }
        if fire(sc(cmd, Key::Minus)) {
            self.map.zoom_keyboard(-1, ppp);
        }
        if fire(sc(cmd, Key::Num0)) {
            let z = self.map.zoom;
            self.map.zoom_keyboard(-z, ppp);
        }
        // Alt + letter selects a tool, as in WorldPainter's tool palette.
        for info in ToolChoice::WORLDPAINTER.iter() {
            if let Some(k) = info.key
                && fire(sc(Modifiers::ALT, k))
            {
                self.tool = info.tool;
                self.status = format!("{} tool", self.tool.name());
            }
        }
        if typing {
            return;
        }
        let in_3d = self.view_mode == ViewMode::View3D || (self.view_mode == ViewMode::Split && self.hover_3d);
        let pressed = ctx.input(|i| {
            let m = i.modifiers;
            if m.alt || m.ctrl || m.command || m.mac_cmd {
                return Vec::new();
            }
            // "+" needs Shift on most layouts, so Shift is allowed for the zoom keys only.
            let zoom_keys = [Key::Minus, Key::Plus, Key::Equals];
            let keys = [
                Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9, Key::Num0,
                Key::R, Key::L, Key::Escape, Key::OpenBracket, Key::CloseBracket, Key::Tab, Key::F, Key::F1, Key::Minus, Key::Plus, Key::Equals,
            ];
            keys.into_iter().filter(|k| i.key_pressed(*k) && (!m.shift || zoom_keys.contains(k))).collect::<Vec<Key>>()
        });
        for k in pressed {
            match k {
                // 3D view: 0 resets the camera (WorldPainter's 3D view).
                Key::Num0 if in_3d => {
                    if let Some(v) = &mut self.view3d {
                        v.reset_view();
                    }
                }
                // WorldPainter: 1 = "10 %" ... 9 = "90 %", 0 = 100 % intensity (its exact values).
                Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9 | Key::Num0 => {
                    const LEVELS: [f32; 10] = [2.0, 16.0, 23.0, 37.0, 51.0, 58.0, 65.0, 79.0, 86.0, 100.0];
                    let idx = match k {
                        Key::Num1 => 0, Key::Num2 => 1, Key::Num3 => 2, Key::Num4 => 3, Key::Num5 => 4,
                        Key::Num6 => 5, Key::Num7 => 6, Key::Num8 => 7, Key::Num9 => 8, _ => 9,
                    };
                    self.brush.intensity = LEVELS[idx] / 100.0;
                    self.status = format!("Intensity {}%", if idx == 9 { 100 } else { (idx + 1) * 10 });
                }
                Key::R | Key::L => {
                    let clockwise = k == Key::R;
                    if in_3d {
                        if let Some(v) = &mut self.view3d {
                            v.rotate_90(clockwise);
                        }
                    } else {
                        self.settings.light = if clockwise { self.settings.light.right() } else { self.settings.light.left() };
                        self.status = format!("Light from the {}", self.settings.light.name());
                    }
                }
                Key::Minus | Key::Plus | Key::Equals if in_3d => {
                    if let Some(v) = &mut self.view3d {
                        v.zoom_step(k != Key::Minus);
                    }
                }
                Key::Escape => {
                    self.tool = ToolChoice::None;
                    self.status = "No tool selected".into();
                }
                Key::OpenBracket => self.brush.radius = (self.brush.radius / 1.1).floor().max(0.0),
                Key::CloseBracket => self.brush.radius = (self.brush.radius * 1.1).ceil().min(crate::map2d::MAX_RADIUS),
                Key::Tab => {
                    self.view_mode = match self.view_mode {
                        ViewMode::Map => ViewMode::View3D,
                        ViewMode::View3D => ViewMode::Split,
                        ViewMode::Split => ViewMode::Map,
                    };
                }
                Key::F if !in_3d => self.frame_map = true,
                Key::F1 => self.dialogs.help = true,
                _ => {}
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New world…").clicked() {
                    self.dialogs.new_world.open = true;
                    ui.close();
                }
                if ui.button("Open…").clicked() {
                    self.dialogs.open_file = true;
                    ui.close();
                }
                ui.menu_button("Open recent", |ui| {
                    let recent = self.settings.recent_files.clone();
                    if recent.is_empty() {
                        ui.label("No recent worlds");
                    }
                    for p in recent {
                        if ui.button(p.display().to_string()).clicked() {
                            self.open_world(&p);
                            ui.close();
                        }
                    }
                });
                if ui.button("Save").clicked() {
                    self.save_world(None);
                    ui.close();
                }
                if ui.button("Save as…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Meadow world", &["meadow"]).set_file_name(format!("{}.meadow", self.world.name)).save_file() {
                        self.save_world(Some(p));
                    }
                    ui.close();
                }
                ui.separator();
                ui.menu_button("Import", |ui| {
                    if ui.button("Height map…").clicked() {
                        self.dialogs.import.pick_file();
                        ui.close();
                    }
                    if ui.button("Mask from image…").clicked() {
                        self.dialogs.mask_from_image();
                        ui.close();
                    }
                    if ui.button("Mod / Minecraft jar blocks (e.g. Conquest Reforged)…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Jar", &["jar", "zip"]).pick_file() {
                            self.jar_job = Some(JarJob::start(p, ui.ctx().clone()));
                        }
                        ui.close();
                    }
                });
                ui.menu_button("Export", |ui| {
                    if ui.button("Minecraft world…").clicked() {
                        self.dialogs.export.open = true;
                        ui.close();
                    }
                    if ui.button("Height map (16-bit PNG)…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("PNG", &["png"]).save_file() {
                            match meadow_core::heightmap::export_png16(self.dim(), &p) {
                                Ok(()) => self.toast(format!("Wrote {}", p.display())),
                                Err(e) => self.error(format!("{e:#}")),
                            }
                        }
                        ui.close();
                    }
                });
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let un = self.dim().undo_name().map(|s| format!("Undo {s}")).unwrap_or_else(|| "Undo".into());
                if ui.add_enabled(self.dim().undo_name().is_some(), egui::Button::new(un).shortcut_text("Ctrl+Z")).clicked() {
                    self.undo();
                    ui.close();
                }
                let rn = self.dim().redo_name().map(|s| format!("Redo {s}")).unwrap_or_else(|| "Redo".into());
                if ui.add_enabled(self.dim().redo_name().is_some(), egui::Button::new(rn).shortcut_text("Ctrl+Y")).clicked() {
                    self.redo();
                    ui.close();
                }
                ui.separator();
                if ui.button("World settings…").clicked() {
                    self.dialogs.world_settings.open_for(&self.world);
                    ui.close();
                }
                if ui.button("Preferences…").clicked() {
                    self.dialogs.settings_open = true;
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                ui.radio_value(&mut self.view_mode, ViewMode::Map, "2D map (Tab)");
                ui.radio_value(&mut self.view_mode, ViewMode::View3D, "3D view");
                ui.radio_value(&mut self.view_mode, ViewMode::Split, "Split 2D | 3D");
                ui.separator();
                if ui.button("Frame world (F)").clicked() {
                    self.frame_map = true;
                    if let Some(v) = &mut self.view3d {
                        v.scene.frame_world();
                    }
                    ui.close();
                }
                ui.checkbox(&mut self.settings.show_grid, "Grid");
                ui.checkbox(&mut self.settings.contours, "Contours");
                ui.checkbox(&mut self.settings.show_layer_overlays, "Overlays (layers)");
                ui.checkbox(&mut self.settings.show_biomes, "Show biomes");
                ui.checkbox(&mut self.mask_overlay, "Show active mask");
                if ui.add(egui::Button::new("Rotate light anticlockwise").shortcut_text("L")).clicked() {
                    self.settings.light = self.settings.light.left();
                }
                if ui.add(egui::Button::new("Rotate light clockwise").shortcut_text("R")).clicked() {
                    self.settings.light = self.settings.light.right();
                }
                ui.menu_button("Change colour scheme", |ui| {
                    let current = self.settings.colors_from_jar.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "built-in colours".into());
                    ui.label(egui::RichText::new(format!("Now: {current}")).small().weak());
                    if ui.button("Colours from installed Minecraft").clicked() {
                        match meadow_blocks::modjar::find_minecraft_jar() {
                            Some(jar) => self.jar_job = Some(JarJob::start(jar, ui.ctx().clone())),
                            None => self.error("Couldn't find a Minecraft installation (.minecraft/versions). Use \"Colours from a jar…\" to pick the client jar."),
                        }
                        ui.close();
                    }
                    if ui.button("Colours from a jar…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().add_filter("Minecraft or mod jar", &["jar", "zip"]).pick_file() {
                            self.jar_job = Some(JarJob::start(p, ui.ctx().clone()));
                        }
                        ui.close();
                    }
                });
                if ui.add(egui::Button::new("Configure view…").shortcut_text("Ctrl+V")).clicked() {
                    self.dialogs.configure_view = true;
                    ui.close();
                }
                if ui.button("Move to spawn").clicked() {
                    self.map.center = (self.world.spawn.0 as f64, self.world.spawn.1 as f64);
                    ui.close();
                }
                if ui.button("Move to origin").clicked() {
                    self.map.center = (0.0, 0.0);
                    ui.close();
                }
                ui.separator();
                ui.menu_button("Panels", |ui| {
                    for p in &mut self.settings.panels {
                        let mut visible = p.dock != Dock::Hidden;
                        if ui.checkbox(&mut visible, p.panel.title()).changed() {
                            p.dock = if visible { if matches!(p.panel, PanelKind::Tools | PanelKind::Brush | PanelKind::Info) { Dock::Left } else { Dock::Right } } else { Dock::Hidden };
                            p.open = visible;
                        }
                    }
                });
                ui.menu_button("UI scale", |ui| {
                    for s in [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0] {
                        if ui.radio(self.settings.ui_scale == s, format!("{:.0}%", s * 100.0)).clicked() {
                            self.settings.ui_scale = s;
                            self.settings_applied = false;
                            self.settings.save();
                        }
                    }
                });
            });
            ui.menu_button("Tools", |ui| {
                if ui.button("Rescale heights vertically…").clicked() {
                    self.dialogs.rescale.open = true;
                    ui.close();
                }
                if ui.button("Resize world horizontally…").clicked() {
                    self.dialogs.resize.open = true;
                    ui.close();
                }
                if ui.button("Remap height range…").clicked() {
                    let snap = self.dim().snapshot();
                    self.dialogs.remap.open_for(&snap);
                    ui.close();
                }
                if ui.button("Fill / clear through mask…").clicked() {
                    self.dialogs.fill.open = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("Run script…").clicked() {
                    self.dialogs.script.open = true;
                    ui.close();
                }
                if ui.button("Block browser…").clicked() {
                    self.dialogs.blocks.open = true;
                    ui.close();
                }
            });
            ui.menu_button("Dimension", |ui| {
                for kind in [DimensionKind::Overworld, DimensionKind::Nether, DimensionKind::End] {
                    let exists = self.world.dimension(kind).is_some();
                    let label = if exists { kind.name().to_string() } else { format!("Create {}…", kind.name()) };
                    if ui.radio(self.dim == kind, label).clicked() {
                        if !exists {
                            self.dialogs.new_dimension = Some(kind);
                        } else {
                            self.dim = kind;
                            self.frame_map = true;
                        }
                        ui.close();
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard & mouse").clicked() {
                    self.dialogs.help = true;
                    ui.close();
                }
                if ui.button("About Meadow").clicked() {
                    self.dialogs.about = true;
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(&mut self.view_mode, ViewMode::Split, "Split");
                ui.selectable_value(&mut self.view_mode, ViewMode::View3D, "3D");
                ui.selectable_value(&mut self.view_mode, ViewMode::Map, "2D");
                ui.separator();
                let title = format!("{}{}", self.world.name, if self.world.dirty { " •" } else { "" });
                ui.label(egui::RichText::new(title).strong());
            });
        });
    }

    /// The column under the cursor in the active view.
    pub fn hover_column(&self) -> Option<(i32, i32)> {
        match self.view_mode {
            ViewMode::View3D => self.view3d.as_ref().and_then(|v| v.hover).map(|(x, z, _)| (x, z)),
            ViewMode::Split if self.hover_3d => self.view3d.as_ref().and_then(|v| v.hover).map(|(x, z, _)| (x, z)),
            _ => self.map.hover,
        }
    }

    /// WorldPainter's status bar: location, height, slope, material, fluid, biome, radius, zoom.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let d = self.dim();
            let mut fields: Vec<String> = Vec::new();
            match self.hover_column() {
                Some((x, y)) => {
                    fields.push(format!("Location: {x}, {y}"));
                    match d.height(x, y) {
                        Some(h) => {
                            let ih = (h + 0.5).floor() as i32;
                            fields.push(format!("Height: {ih} of {}", d.limits.max_y - 1));
                            fields.push(format!("Slope: {:.0}°", d.slope(x, y)));
                            fields.push(format!("Material: {}", self.world.terrain(d.terrain(x, y).unwrap_or(0)).name));
                            let w = d.water(x, y).unwrap_or(i32::MIN);
                            if w > ih {
                                let lava = d.layer_value(LayerId::LAVA, 0, x, y) != 0;
                                fields.push(format!("{} level: {w} (depth {})", if lava { "Lava" } else { "Fluid" }, w - ih));
                            }
                            let b = d.layer_value(LayerId::BIOME, meadow_blocks::BIOME_AUTO, x, y);
                            let biomes = meadow_blocks::biomes();
                            if b != meadow_blocks::BIOME_AUTO {
                                if let Some(bi) = biomes.by_numeric(b) {
                                    fields.push(format!("Biome: {} (ID {b})", bi.name));
                                }
                            } else {
                                let auto = self.world.terrain(d.terrain(x, y).unwrap_or(0)).auto_biome;
                                if let Some(bi) = biomes.by_numeric(auto) {
                                    fields.push(format!("Auto biome: {} (ID {auto})", bi.name));
                                }
                            }
                        }
                        None => fields.push("No tile".into()),
                    }
                }
                None => fields.push("Location: -".into()),
            }
            fields.push(format!("Radius: {}", self.brush.radius.round() as i32));
            fields.push(format!("Zoom: {}%", self.map.zoom_percent()));
            for (k, f) in fields.iter().enumerate() {
                if k > 0 {
                    ui.separator();
                }
                ui.label(f);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(self.world.export.version.label()).weak());
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(&self.status);
                }
            });
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let mode = if self.gpu.is_none() { ViewMode::Map } else { self.view_mode };
        match mode {
            ViewMode::Map => self.map_view(ui),
            ViewMode::View3D => self.view_3d(ui),
            ViewMode::Split => {
                let w = ui.available_width();
                let h = ui.available_height();
                ui.horizontal(|ui| {
                    ui.allocate_ui(egui::vec2(w * 0.5 - 4.0, h), |ui| self.map_view(ui));
                    ui.separator();
                    ui.allocate_ui(egui::vec2(w * 0.5 - 4.0, h), |ui| self.view_3d(ui));
                });
            }
        }
    }

    /// Layers drawn on the 2D map, in drawing order (WorldPainter draws every layer unless its
    /// "show" box is unticked).
    pub fn visible_layers(&self) -> Vec<LayerId> {
        if !self.settings.show_layer_overlays {
            return Vec::new();
        }
        let mut v: Vec<&LayerDef> = self
            .world
            .layers
            .iter()
            .filter(|l| !self.hidden_layers.contains(&l.id))
            .filter(|l| l.id != LayerId::BIOME || self.settings.show_biomes)
            .collect();
        v.sort_by_key(|l| l.priority);
        v.into_iter().map(|l| l.id).collect()
    }

    fn map_view(&mut self, ui: &mut egui::Ui) {
        let style = MapStyle {
            light: self.settings.light,
            hillshade: self.settings.hillshade,
            contours: self.settings.contours,
            contour_spacing: self.settings.contour_spacing,
            layers: self.visible_layers(),
            mask: if self.mask_overlay { self.mask_filter.and_then(|i| self.world.masks.get(i).cloned()) } else { None },
            background: self.settings.background,
            defs_gen: self.defs_gen,
            colors_gen: meadow_blocks::registry().read().generation(),
        };
        let ppp = ui.ctx().pixels_per_point();
        if self.frame_map {
            let size = ui.available_size();
            let kind = self.dim;
            if let Some(d) = self.world.dimension(kind) {
                self.map.frame_world(d, size, ppp);
            }
            self.frame_map = false;
        }
        let kind = self.dim;
        let Some(dim) = self.world.dimension(kind) else { return };
        let action = self.map.ui(
            ui,
            MapUi {
                world: &self.world,
                dim,
                style: &style,
                brush: &mut self.brush,
                show_grid: self.settings.show_grid,
                grid_size: self.settings.grid_size,
                click_tool: self.tool.is_click(),
                tool_active: self.tool != ToolChoice::None,
                brush_tool: self.tool.uses_brush(),
            },
        );
        match action {
            MapAction::Paint { from, to, dt, first, inverse } => self.paint(from, to, dt, first, inverse),
            MapAction::EndPaint => self.end_paint(),
            MapAction::Click { x, y, inverse } => self.click(x, y, inverse),
            MapAction::None => {}
        }
    }

    fn view_3d(&mut self, ui: &mut egui::Ui) {
        let (Some(gpu), Some(v)) = (&self.gpu, &mut self.view3d) else {
            ui.centered_and_justified(|ui| ui.label("3D view needs a GPU (wgpu) renderer"));
            return;
        };
        let editing = self.map.painting.is_some();
        v.ui(ui, &gpu.rs, &gpu.r, &self.world, self.dim, self.defs_gen, &self.settings, editing);
        self.hover_3d = v.hovered;
    }

    fn side_panels(&mut self, ui: &mut egui::Ui) {
        let panels = self.settings.panels.clone();
        let left: Vec<_> = panels.iter().filter(|p| p.dock == Dock::Left).cloned().collect();
        let right: Vec<_> = panels.iter().filter(|p| p.dock == Dock::Right).cloned().collect();
        if !left.is_empty() {
            let r = egui::Panel::left("left").resizable(true).default_size(self.settings.left_width).min_size(180.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("left_scroll").show(ui, |ui| {
                    for p in &left {
                        self.panel_section(ui, p.panel, p.open);
                    }
                });
            });
            self.settings.left_width = r.response.rect.width();
        }
        if !right.is_empty() {
            let r = egui::Panel::right("right").resizable(true).default_size(self.settings.right_width).min_size(200.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("right_scroll").show(ui, |ui| {
                    for p in &right {
                        self.panel_section(ui, p.panel, p.open);
                    }
                });
            });
            self.settings.right_width = r.response.rect.width();
        }
    }

    fn panel_section(&mut self, ui: &mut egui::Ui, kind: PanelKind, default_open: bool) {
        let resp = egui::CollapsingHeader::new(egui::RichText::new(kind.title()).strong())
            .id_salt(("panel", kind as u8))
            .default_open(default_open)
            .show(ui, |ui| match kind {
                PanelKind::Tools => self.panel_tools(ui),
                PanelKind::Brush => self.panel_brush(ui),
                PanelKind::Terrain => self.panel_terrain(ui),
                PanelKind::Layers => self.panel_layers(ui),
                PanelKind::Masks => self.panel_masks(ui),
                PanelKind::Biomes => self.panel_biomes(ui),
                PanelKind::Info => self.panel_info(ui),
            });
        resp.header_response.context_menu(|ui| {
            for (label, dock) in [("Move to left", Dock::Left), ("Move to right", Dock::Right), ("Hide", Dock::Hidden)] {
                if ui.button(label).clicked() {
                    if let Some(p) = self.settings.panels.iter_mut().find(|p| p.panel == kind) {
                        p.dock = dock;
                    }
                    self.settings.save();
                    ui.close();
                }
            }
            if ui.button("Move up").clicked() {
                if let Some(i) = self.settings.panels.iter().position(|p| p.panel == kind)
                    && i > 0
                {
                    self.settings.panels.swap(i, i - 1);
                }
                ui.close();
            }
            if ui.button("Move down").clicked() {
                if let Some(i) = self.settings.panels.iter().position(|p| p.panel == kind)
                    && i + 1 < self.settings.panels.len()
                {
                    self.settings.panels.swap(i, i + 1);
                }
                ui.close();
            }
        });
        ui.add_space(4.0);
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.export_job
            && let Ok(r) = j.rx.try_recv()
        {
            let dir = j.dir.clone();
            self.export_job = None;
            match r {
                Ok(stats) => {
                    self.dialogs.export.result = Some(stats.clone());
                    self.toast(format!("Exported {} chunks to {} in {:.1}s", stats.chunks, dir.display(), stats.elapsed.as_secs_f32()));
                }
                Err(e) => self.error(format!("Export failed:\n{e}")),
            }
        }
        if let Some(j) = &mut self.script_job {
            while let Ok(s) = j.output_rx.try_recv() {
                j.output.push_str(&s);
            }
            if let Ok(outcome) = j.done_rx.try_recv() {
                while let Ok(s) = j.output_rx.try_recv() {
                    j.output.push_str(&s);
                }
                let mut log = std::mem::take(&mut j.output);
                let name = j.name.clone();
                let secs = j.started.elapsed().as_secs_f32();
                self.script_job = None;
                if let Some(w) = outcome.world {
                    let path = self.world.path.clone();
                    self.world = w;
                    self.world.path = path;
                    self.defs_changed();
                }
                if let Some(e) = &outcome.error {
                    log.push_str(&format!("\nError: {e}\n"));
                }
                log.push_str(&format!("\n{name} finished in {secs:.2}s (exit code {})\n", outcome.exit_code));
                for w in outcome.other_worlds {
                    log.push_str(&format!("Script created world \"{}\" (save it from the script with wp.saveWorld to keep it)\n", w.name));
                }
                self.dialogs.script.output = log;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(j) = &self.jar_job
            && let Ok(r) = j.rx.try_recv()
        {
            self.jar_job = None;
            match r {
                Ok((path, blocks)) => {
                    crate::jobs::save_imported_blocks(&blocks);
                    let rep = meadow_blocks::modjar::import_into_registry(blocks, &[]);
                    let ns: Vec<String> = rep.namespaces.iter().map(|(n, c)| format!("{n}: {c}")).collect();
                    let vanilla = rep.namespaces.iter().all(|(n, _)| n == "minecraft");
                    if vanilla {
                        self.toast(format!("Block colours now come from {}", path.display()));
                        self.settings.colors_from_jar = Some(path.clone());
                    } else {
                        self.toast(format!("Imported {} blocks from {} ({})", rep.blocks, path.display(), ns.join(", ")));
                    }
                    self.map.clear_cache();
                    self.settings.imported_jars.push(path);
                    self.settings.save();
                    self.defs_changed();
                }
                Err(e) => self.error(format!("Jar import failed:\n{e}")),
            }
        }
    }

    fn autosave(&mut self) {
        let mins = self.settings.autosave_minutes;
        if mins == 0 || !self.world.dirty || self.map.painting.is_some() || self.last_autosave.elapsed().as_secs() < mins as u64 * 60 {
            return;
        }
        self.last_autosave = Instant::now();
        if let Some(dir) = crate::settings::config_dir() {
            let p = dir.join("autosave.meadow");
            let _ = std::fs::create_dir_all(&dir);
            let mut snap = self.world.snapshot();
            if snap.save(&p).is_ok() {
                self.status = format!("Autosaved to {}", p.display());
            }
        }
    }
}

impl eframe::App for MeadowApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.settings_applied {
            self.settings.apply(&ctx);
            self.settings_applied = true;
        }
        self.poll_jobs(&ctx);
        self.shortcuts(&ctx);
        if self.dialogs.open_file {
            self.dialogs.open_file = false;
            if let Some(p) = rfd::FileDialog::new().add_filter("Meadow world", &["meadow"]).pick_file() {
                self.open_world(&p);
            }
        }
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        self.side_panels(ui);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| self.central(ui));
        self.show_dialogs(&ctx);
        // Toasts
        self.toasts.retain(|(_, t)| t.elapsed().as_secs() < 6);
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::unique("toasts")).anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -36.0)).show(&ctx, |ui| {
                for (m, _) in &self.toasts {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(m);
                    });
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.autosave();
        let title = format!("Meadow — {}{}", self.world.name, if self.world.dirty { " *" } else { "" });
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn on_exit(&mut self) {
        self.settings.save();
    }
}
