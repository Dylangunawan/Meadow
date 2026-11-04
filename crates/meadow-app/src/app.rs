//! The Meadow editor application.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers};
use meadow_core::layer::{LayerDef, LayerId};
use meadow_core::ops::{self, Brush, BrushShape, Stroke, Tool};
use meadow_core::world::NewWorld;
use meadow_core::{DimensionKind, World};
use meadow_render::Renderer;

use crate::dialogs::Dialogs;
use crate::jobs::{ExportJob, JarJob, ScriptJob};
use crate::map2d::{Map2D, MapAction, MapStyle};
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
    Raise,
    Lower,
    Smooth,
    Flatten,
    SetHeight,
    Roughen,
    Terrace,
    Terrain,
    Layer,
    EraseLayer,
    Biome,
    WaterLevel,
    Sponge,
    Flood,
    Mask,
    Tiles,
    Spawn,
}

impl ToolChoice {
    pub const ALL: [(ToolChoice, &'static str, &'static str); 17] = [
        (ToolChoice::Raise, "Raise", "Raise the terrain (1)"),
        (ToolChoice::Lower, "Lower", "Lower the terrain (2)"),
        (ToolChoice::Smooth, "Smooth", "Smooth the terrain (3)"),
        (ToolChoice::Flatten, "Flatten", "Flatten to the height where the stroke started (4)"),
        (ToolChoice::SetHeight, "Set height", "Move the terrain towards a fixed height (5)"),
        (ToolChoice::Roughen, "Roughen", "Add noise to the terrain"),
        (ToolChoice::Terrace, "Terrace", "Shape the terrain into terraces"),
        (ToolChoice::Terrain, "Paint terrain", "Paint the selected terrain type (6)"),
        (ToolChoice::Layer, "Paint layer", "Paint the selected layer (7)"),
        (ToolChoice::EraseLayer, "Erase layer", "Remove the selected layer (8)"),
        (ToolChoice::Biome, "Paint biome", "Paint the selected biome (9)"),
        (ToolChoice::WaterLevel, "Water level", "Set the water level under the brush"),
        (ToolChoice::Sponge, "Sponge", "Remove water under the brush"),
        (ToolChoice::Flood, "Flood", "Click to flood a basin up to the water level"),
        (ToolChoice::Mask, "Paint mask", "Paint the selected painted mask (shift erases)"),
        (ToolChoice::Tiles, "Add/remove tiles", "Extend the world (shift removes tiles)"),
        (ToolChoice::Spawn, "Spawn point", "Click to set the spawn point"),
    ];
    pub fn is_click(self) -> bool {
        matches!(self, ToolChoice::Flood | ToolChoice::Spawn)
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
    pub terrain: u8,
    pub layer: Option<LayerId>,
    pub layer_value: u8,
    pub biome: u8,
    pub height_level: f32,
    pub water_level: i32,
    pub mask_filter: Option<usize>,
    pub mask_overlay: bool,
    pub paint_mask_layer: Option<LayerId>,
    pub overlays: Vec<LayerId>,
    pub stroke: Stroke,
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
            let r = Renderer::new(rs.device.clone(), rs.queue.clone());
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
            tool: ToolChoice::Raise,
            brush: Brush::default(),
            terrain: 0,
            layer: Some(LayerId::DECIDUOUS),
            layer_value: 10,
            biome: 1,
            height_level: 64.0,
            water_level: 62,
            mask_filter: None,
            mask_overlay: false,
            paint_mask_layer: None,
            overlays: vec![LayerId::DECIDUOUS, LayerId::PINE, LayerId::JUNGLE, LayerId::SWAMP, LayerId::FROST, LayerId::ANNOTATIONS],
            stroke: Stroke::default(),
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
            app.toast(format!("Loaded {imported} imported mod blocks"));
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

    fn core_tool(&self, erase: bool) -> Option<Tool> {
        let layer_def = |id: LayerId| self.world.layer(id).cloned();
        Some(match self.tool {
            ToolChoice::Raise => Tool::Raise,
            ToolChoice::Lower => Tool::Lower,
            ToolChoice::Smooth => Tool::Smooth,
            ToolChoice::Flatten => Tool::Flatten,
            ToolChoice::SetHeight => Tool::SetHeight(self.height_level),
            ToolChoice::Roughen => Tool::Roughen,
            ToolChoice::Terrace => Tool::Terrace(8.0),
            ToolChoice::Terrain => Tool::PaintTerrain(self.terrain),
            ToolChoice::Layer | ToolChoice::EraseLayer => {
                let d: LayerDef = layer_def(self.layer?)?;
                if self.tool == ToolChoice::EraseLayer || erase {
                    Tool::EraseLayer { layer: d.id, bits: d.is_bits(), default: d.default }
                } else {
                    Tool::PaintLayer { layer: d.id, value: self.layer_value.min(d.data.max()).max(1), bits: d.is_bits(), default: d.default }
                }
            }
            ToolChoice::Biome => {
                if erase {
                    Tool::EraseLayer { layer: LayerId::BIOME, bits: false, default: meadow_blocks::BIOME_AUTO }
                } else {
                    Tool::PaintBiome(self.biome)
                }
            }
            ToolChoice::WaterLevel => Tool::SetWaterLevel(self.water_level),
            ToolChoice::Sponge => Tool::Sponge,
            ToolChoice::Mask => {
                let l = self.paint_mask_layer?;
                if erase { Tool::EraseMask(l) } else { Tool::PaintMask(l) }
            }
            ToolChoice::Flood | ToolChoice::Tiles | ToolChoice::Spawn => return None,
        })
    }

    fn paint(&mut self, from: (f32, f32), to: (f32, f32), dt: f32, first: bool, erase: bool) {
        let tool = self.core_tool(erase);
        let tiles_tool = self.tool == ToolChoice::Tiles;
        if tool.is_none() && !tiles_tool {
            if matches!(self.tool, ToolChoice::Layer | ToolChoice::EraseLayer) {
                self.status = "Select a layer to paint in the Layers panel".into();
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
                        if erase {
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

    fn click(&mut self, x: i32, y: i32, secondary: bool) {
        match self.tool {
            ToolChoice::Flood => {
                let level = self.water_level;
                let d = self.dim_mut();
                d.begin_op("Flood");
                let _ = secondary;
                let n = ops::flood(d, x, y, level, 4_000_000);
                d.end_op();
                self.status = format!("Flooded {n} columns to level {level}");
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

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
        let fire = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if fire(sc(Modifiers::COMMAND, Key::Z)) {
            self.undo();
        }
        if fire(sc(Modifiers::COMMAND, Key::Y)) || fire(sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) {
            self.redo();
        }
        if fire(sc(Modifiers::COMMAND, Key::S)) {
            self.save_world(None);
        }
        if fire(sc(Modifiers::COMMAND, Key::O)) {
            self.dialogs.open_file = true;
        }
        if fire(sc(Modifiers::COMMAND, Key::N)) {
            self.dialogs.new_world.open = true;
        }
        if fire(sc(Modifiers::COMMAND, Key::E)) {
            self.dialogs.export.open = true;
        }
        if typing {
            return;
        }
        ctx.input(|i| {
            let nums = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
            let tools = [
                ToolChoice::Raise,
                ToolChoice::Lower,
                ToolChoice::Smooth,
                ToolChoice::Flatten,
                ToolChoice::SetHeight,
                ToolChoice::Terrain,
                ToolChoice::Layer,
                ToolChoice::EraseLayer,
                ToolChoice::Biome,
            ];
            if !i.modifiers.any() {
                for (k, t) in nums.iter().zip(tools) {
                    if i.key_pressed(*k) {
                        self.tool = t;
                    }
                }
                if i.key_pressed(Key::OpenBracket) {
                    self.brush.radius = (self.brush.radius / 1.15).max(0.5);
                }
                if i.key_pressed(Key::CloseBracket) {
                    self.brush.radius = (self.brush.radius * 1.15).min(500.0);
                }
                if i.key_pressed(Key::Tab) {
                    self.view_mode = match self.view_mode {
                        ViewMode::Map => ViewMode::View3D,
                        ViewMode::View3D => ViewMode::Split,
                        ViewMode::Split => ViewMode::Map,
                    };
                }
                if i.key_pressed(Key::F) && self.view_mode == ViewMode::Map {
                    self.frame_map = true;
                }
            }
        });
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
                ui.checkbox(&mut self.settings.show_grid, "Tile and chunk grid");
                ui.checkbox(&mut self.settings.show_layer_overlays, "Layer overlays");
                ui.checkbox(&mut self.mask_overlay, "Show active mask");
                ui.horizontal(|ui| {
                    ui.label("Contours every");
                    ui.add(egui::DragValue::new(&mut self.settings.contour_interval).range(0..=256).suffix(" blocks"));
                });
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let hover = match self.view_mode {
                ViewMode::View3D => self.view3d.as_ref().and_then(|v| v.hover).map(|(x, z, _)| (x, z)),
                _ => self.map.hover,
            };
            if let Some((x, y)) = hover {
                let d = self.dim();
                match d.height(x, y) {
                    Some(h) => {
                        let t = self.world.terrain(d.terrain(x, y).unwrap_or(0)).name.clone();
                        let w = d.water(x, y).unwrap_or(0);
                        let mut s = format!("x {x}  z {y}  height {h:.1}  {t}  slope {:.0}°", d.slope(x, y));
                        if w > h.round() as i32 {
                            s += &format!("  water {w} (depth {})", w - h.round() as i32);
                        }
                        let b = d.layer_value(LayerId::BIOME, meadow_blocks::BIOME_AUTO, x, y);
                        if b != meadow_blocks::BIOME_AUTO
                            && let Some(bi) = meadow_blocks::biomes().by_numeric(b)
                        {
                            s += &format!("  biome {}", bi.name);
                        }
                        let layers: Vec<String> = self
                            .world
                            .layers
                            .iter()
                            .filter(|l| l.id != LayerId::BIOME)
                            .filter_map(|l| {
                                let v = d.layer_value(l.id, l.default, x, y);
                                (v != l.default).then(|| if l.is_bits() { l.name.clone() } else { format!("{} {v}", l.name) })
                            })
                            .collect();
                        if !layers.is_empty() {
                            s += &format!("  [{}]", layers.join(", "));
                        }
                        ui.monospace(s);
                    }
                    None => {
                        ui.monospace(format!("x {x}  z {y}  (no tile)"));
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let d = self.dim();
                ui.label(format!(
                    "{} tiles · y {}..{} · {}",
                    d.tile_count(),
                    d.limits.min_y,
                    d.limits.max_y,
                    self.world.export.version.label()
                ));
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

    fn map_view(&mut self, ui: &mut egui::Ui) {
        let style = MapStyle {
            hillshade: self.settings.hillshade,
            contour: self.settings.contour_interval,
            overlays: if self.settings.show_layer_overlays { self.overlays.clone() } else { Vec::new() },
            mask: if self.mask_overlay { self.mask_filter.and_then(|i| self.world.masks.get(i).cloned()) } else { None },
            defs_gen: self.defs_gen,
        };
        if self.frame_map {
            let size = ui.available_size();
            let kind = self.dim;
            if let Some(d) = self.world.dimension(kind) {
                self.map.frame_world(d, size);
            }
            self.frame_map = false;
        }
        let kind = self.dim;
        let world = &self.world;
        let Some(dim) = world.dimension(kind) else { return };
        let action = self.map.ui(
            ui,
            world,
            dim,
            &style,
            self.brush.radius,
            self.brush.shape == BrushShape::Square,
            self.settings.show_grid,
            self.tool.is_click(),
        );
        let erase = ui.input(|i| i.modifiers.shift);
        match action {
            MapAction::Paint { from, to, dt, first } => self.paint(from, to, dt, first, erase),
            MapAction::EndPaint => self.end_paint(),
            MapAction::Click { x, y, secondary } => self.click(x, y, secondary),
            MapAction::None => {}
        }
    }

    fn view_3d(&mut self, ui: &mut egui::Ui) {
        let (Some(gpu), Some(v)) = (&self.gpu, &mut self.view3d) else {
            ui.centered_and_justified(|ui| ui.label("3D view needs a GPU (wgpu) renderer"));
            return;
        };
        let editing = self.map.painting;
        v.ui(ui, &gpu.rs, &gpu.r, &self.world, self.dim, self.defs_gen, &self.settings, editing);
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
                    self.toast(format!("Imported {} blocks from {} ({})", rep.blocks, path.display(), ns.join(", ")));
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
        if mins == 0 || !self.world.dirty || self.map.painting || self.last_autosave.elapsed().as_secs() < mins as u64 * 60 {
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
