//! 3D viewport and object preview widgets (GPU scenes shown as egui images).

use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{self, Sense};
use meadow_core::layer::LayerDef;
use meadow_core::{DimensionKind, World};
use meadow_render::preview::{PreviewOptions, preview_world, single_object_layer};
use meadow_render::wgpu;
use meadow_render::{Renderer, Scene};

use crate::settings::Settings;

fn show_scene(ui: &mut egui::Ui, rs: &egui_wgpu::RenderState, r: &Renderer, scene: &mut Scene, tex: &mut Option<egui::TextureId>, size: egui::Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = (((rect.width() * ppp) as u32).max(16), ((rect.height() * ppp) as u32).max(16));
    let view = scene.render(r, w, h);
    {
        let mut ren = rs.renderer.write();
        match *tex {
            None => *tex = Some(ren.register_native_texture(&rs.device, view, wgpu::FilterMode::Linear)),
            Some(id) => ren.update_egui_texture_from_wgpu_texture(&rs.device, view, wgpu::FilterMode::Linear, id),
        }
    }
    if let Some(id) = *tex {
        ui.painter().image(id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    resp
}

/// Orbit with left drag, pan with right/middle drag, zoom with the wheel, fly with WASD/QE.
fn camera_controls(ui: &egui::Ui, resp: &egui::Response, scene: &mut Scene, fly_speed: f32) -> bool {
    let mut moved = false;
    if resp.dragged_by(egui::PointerButton::Primary) {
        let d = resp.drag_delta();
        scene.camera.orbit(d.x, d.y);
        moved = true;
    }
    if resp.dragged_by(egui::PointerButton::Secondary) || resp.dragged_by(egui::PointerButton::Middle) {
        let d = resp.drag_delta();
        scene.camera.pan(d.x, d.y);
        moved = true;
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
        if scroll != 0.0 {
            scene.camera.zoom(scroll / 60.0);
            moved = true;
        }
        let (dt, keys) = ui.input(|i| {
            let k = |key| if i.key_down(key) { 1.0f32 } else { 0.0 };
            (
                i.stable_dt.min(0.1),
                (k(egui::Key::W) - k(egui::Key::S), k(egui::Key::D) - k(egui::Key::A), k(egui::Key::E) - k(egui::Key::Q), i.modifiers.shift),
            )
        });
        let (f, r, u, fast) = keys;
        if f != 0.0 || r != 0.0 || u != 0.0 {
            let speed = fly_speed * if fast { 4.0 } else { 1.0 } * dt * (scene.camera.distance / 200.0).clamp(0.5, 20.0);
            scene.camera.fly(f * speed, r * speed, u * speed);
            moved = true;
        }
    }
    moved
}

pub struct View3D {
    pub scene: Scene,
    tex: Option<egui::TextureId>,
    last_rev: u64,
    last_gen: u64,
    last_kind: Option<DimensionKind>,
    last_snapshot: Instant,
    framed: bool,
    pub hover: Option<(i32, i32, f32)>,
}

impl View3D {
    pub fn new(r: &Renderer) -> View3D {
        View3D {
            scene: Scene::new(r),
            tex: None,
            last_rev: 0,
            last_gen: u64::MAX,
            last_kind: None,
            last_snapshot: Instant::now(),
            framed: false,
            hover: None,
        }
    }

    pub fn apply_settings(&mut self, s: &Settings) {
        let st = &mut self.scene.settings;
        st.msaa = if s.msaa { 4 } else { 1 };
        st.view_distance = s.view_distance;
        st.detail_distance = s.detail_distance;
        st.decoration_distance = s.decoration_distance;
        st.fog = s.fog;
        st.mesh.plants = s.show_plants_3d;
    }

    pub fn refocus(&mut self) {
        self.framed = false;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ui(&mut self, ui: &mut egui::Ui, rs: &egui_wgpu::RenderState, r: &Renderer, world: &World, kind: DimensionKind, defs_gen: u64, settings: &Settings, editing: bool) {
        let rev = world.content_rev();
        let stale = rev != self.last_rev || defs_gen != self.last_gen || self.last_kind != Some(kind);
        // While painting, refresh at most ~8 times per second to keep the editor fluid.
        if stale && (!editing || self.last_snapshot.elapsed().as_millis() > 120) {
            if defs_gen != self.last_gen && self.last_gen != u64::MAX {
                self.scene.invalidate_all();
            }
            self.scene.set_world(Arc::new(world.snapshot()), kind);
            self.last_rev = rev;
            self.last_gen = defs_gen;
            self.last_kind = Some(kind);
            self.last_snapshot = Instant::now();
            if !self.framed {
                self.scene.frame_world();
                self.framed = true;
            }
        }
        self.apply_settings(settings);
        let size = ui.available_size();
        let resp = show_scene(ui, rs, r, &mut self.scene, &mut self.tex, size);
        let moved = camera_controls(ui, &resp, &mut self.scene, settings.fly_speed);
        self.hover = None;
        if let Some(p) = resp.hover_pos() {
            let rect = resp.rect;
            let ndc = ((p.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (p.y - rect.top()) / rect.height() * 2.0);
            self.hover = self.scene.pick(ndc.0, ndc.1, rect.width() / rect.height().max(1.0));
            if resp.double_clicked()
                && let Some((x, z, h)) = self.hover
            {
                self.scene.camera.target = glam::Vec3::new(x as f32, h, z as f32);
            }
        }
        // Overlay: stats and help
        let st = self.scene.stats;
        let text = format!(
            "{} tiles · {:.1}M tris{}   |   drag: orbit · right-drag: pan · wheel: zoom · WASD/QE: fly · double-click: focus",
            st.tiles_drawn,
            st.triangles as f64 / 1e6,
            if st.pending > 0 { format!(" · meshing {}", st.pending) } else { String::new() }
        );
        ui.painter().text(
            resp.rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            text,
            egui::FontId::proportional(11.0),
            egui::Color32::from_white_alpha(220),
        );
        if moved || self.scene.is_busy() || stale {
            ui.ctx().request_repaint();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// The layer painted on a patch of ground, placed exactly like the exporter would.
    Placement,
    /// One object by itself.
    Single(usize),
}

pub struct ObjectPreview {
    pub scene: Scene,
    tex: Option<egui::TextureId>,
    pub opts: PreviewOptions,
    pub mode: PreviewMode,
    pub dirty: bool,
}

impl ObjectPreview {
    pub fn new(r: &Renderer) -> ObjectPreview {
        let mut scene = Scene::new(r);
        scene.settings.view_distance = 600.0;
        scene.settings.decoration_distance = 600.0;
        scene.settings.detail_distance = 600.0;
        scene.settings.fog = false;
        scene.camera.target = glam::Vec3::new(0.0, 66.0, 0.0);
        scene.camera.distance = 90.0;
        scene.camera.pitch = 0.55;
        ObjectPreview { scene, tex: None, opts: PreviewOptions::default(), mode: PreviewMode::Placement, dirty: true }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, rs: &egui_wgpu::RenderState, r: &Renderer, layer: &LayerDef, size: egui::Vec2) {
        if self.dirty {
            let world = match self.mode {
                PreviewMode::Placement => preview_world(layer, &self.opts),
                PreviewMode::Single(i) => {
                    let obj = match &layer.kind {
                        meadow_core::layer::LayerKind::CustomObjects(s) => s.objects.get(i).map(|e| e.object.clone()),
                        _ => None,
                    };
                    match obj {
                        Some(o) => {
                            let l = single_object_layer(o);
                            preview_world(&l, &PreviewOptions { radius: 0, ..self.opts })
                        }
                        None => preview_world(layer, &self.opts),
                    }
                }
            };
            self.scene.invalidate_all();
            self.scene.set_world(Arc::new(world), DimensionKind::Overworld);
            self.dirty = false;
        }
        let resp = show_scene(ui, rs, r, &mut self.scene, &mut self.tex, size);
        let moved = camera_controls(ui, &resp, &mut self.scene, 30.0);
        if moved || self.scene.is_busy() {
            ui.ctx().request_repaint();
        }
    }
}
