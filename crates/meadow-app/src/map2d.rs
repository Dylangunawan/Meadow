//! The 2D top-down editor view, drawn and controlled the way WorldPainter's map view is:
//! power-of-two zoom, hill shading lit from a rotatable direction, contour lines, surface-block
//! colours, forest stamps, black grid with coordinate labels, dashed brush outline.
//!
//! Mouse: left applies the tool, right (or Alt + left) applies its inverse, middle-drag or
//! Space + drag pans, the wheel changes the brush radius (Shift: by one), Ctrl + wheel zooms and
//! Alt + wheel rotates the brush.

use std::time::Instant;

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke as EStroke, Vec2};
use meadow_blocks::StateInfo;
use meadow_core::layer::{LayerDef, LayerId, LayerKind, TreeKind};
use meadow_core::mask::CustomMask;
use meadow_core::ops::{Brush, BrushShape};
use meadow_core::tile::{TILE_SHIFT, TILE_SIZE, local_index};
use meadow_core::{Dimension, World};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

pub const MIN_ZOOM: i32 = -4;
pub const MAX_ZOOM: i32 = 6;
pub const MAX_RADIUS: f32 = 300.0;

/// Where the map's light comes from (WorldPainter's R / L keys cycle through these).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LightOrigin {
    #[default]
    NorthWest,
    NorthEast,
    SouthEast,
    SouthWest,
    Above,
}

impl LightOrigin {
    pub const ALL: [LightOrigin; 5] = [LightOrigin::NorthWest, LightOrigin::NorthEast, LightOrigin::SouthEast, LightOrigin::SouthWest, LightOrigin::Above];
    /// Clockwise (R).
    pub fn right(self) -> Self {
        match self {
            LightOrigin::NorthWest => LightOrigin::NorthEast,
            LightOrigin::NorthEast => LightOrigin::SouthEast,
            LightOrigin::SouthEast => LightOrigin::SouthWest,
            LightOrigin::SouthWest => LightOrigin::Above,
            LightOrigin::Above => LightOrigin::NorthWest,
        }
    }
    /// Anticlockwise (L).
    pub fn left(self) -> Self {
        match self {
            LightOrigin::NorthWest => LightOrigin::Above,
            LightOrigin::NorthEast => LightOrigin::NorthWest,
            LightOrigin::SouthEast => LightOrigin::NorthEast,
            LightOrigin::SouthWest => LightOrigin::SouthEast,
            LightOrigin::Above => LightOrigin::SouthWest,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            LightOrigin::NorthWest => "north-west",
            LightOrigin::NorthEast => "north-east",
            LightOrigin::SouthEast => "south-east",
            LightOrigin::SouthWest => "south-west",
            LightOrigin::Above => "overhead",
        }
    }
    /// WorldPainter's brighten amount (out of 256) from height deltas to the neighbours.
    #[inline]
    fn brighten(self, d_e: i32, d_w: i32, d_s: i32, d_n: i32, strength: f32) -> i32 {
        let sum = match self {
            LightOrigin::NorthWest => d_e - d_w + d_s - d_n,
            LightOrigin::NorthEast => d_w - d_e + d_s - d_n,
            LightOrigin::SouthEast => d_w - d_e + d_n - d_s,
            LightOrigin::SouthWest => d_e - d_w + d_n - d_s,
            LightOrigin::Above => 0,
        };
        ((sum as f32 * 32.0 * strength) as i32 + 256).max(0)
    }
}

/// Everything that affects how tiles look (hashed into the texture cache key).
#[derive(Clone)]
pub struct MapStyle {
    pub light: LightOrigin,
    /// Multiplier on WorldPainter's shading (1.0 = identical).
    pub hillshade: f32,
    pub contours: bool,
    pub contour_spacing: i32,
    /// Layers to draw, in drawing order.
    pub layers: Vec<LayerId>,
    pub mask: Option<CustomMask>,
    pub background: [u8; 3],
    pub defs_gen: u64,
    pub colors_gen: u64,
}

impl MapStyle {
    fn key(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100000001b3);
        mix(self.light as u64);
        mix((self.hillshade * 100.0) as u64);
        mix(self.contours as u64);
        mix(self.contour_spacing as u64);
        for l in &self.layers {
            mix(l.0 as u64 + 7);
        }
        mix(self.mask.as_ref().map(|m| m.name.len() as u64 + 99).unwrap_or(0));
        mix(u32::from_be_bytes([0, self.background[0], self.background[1], self.background[2]]) as u64);
        mix(self.defs_gen);
        mix(self.colors_gen);
        h
    }
}

struct CachedTile {
    rev: u64,
    style: u64,
    tex: egui::TextureHandle,
}

pub struct Map2D {
    /// World coordinates at the centre of the view.
    pub center: (f64, f64),
    /// Zoom as a power of two: the scale is 2^zoom screen pixels per block.
    pub zoom: i32,
    cache: FxHashMap<(i32, i32, u8), CachedTile>,
    pub hover: Option<(i32, i32)>,
    last_paint: Option<(f32, f32)>,
    last_time: Instant,
    /// `Some(inverse)` while a stroke is in progress.
    pub painting: Option<bool>,
    wheel_accum: f32,
    last_rect: Option<Rect>,
    /// egui may run the UI twice in one frame; input is handled only in the first pass.
    last_input_frame: u64,
}

pub enum MapAction {
    None,
    /// Apply the current brush tool along a segment (world coords) with a time step.
    Paint { from: (f32, f32), to: (f32, f32), dt: f32, first: bool, inverse: bool },
    EndPaint,
    Click { x: i32, y: i32, inverse: bool },
}

/// Options for one frame of the map view.
pub struct MapUi<'a> {
    pub world: &'a World,
    pub dim: &'a Dimension,
    pub style: &'a MapStyle,
    pub brush: &'a mut Brush,
    pub show_grid: bool,
    pub grid_size: i32,
    /// The tool works by clicking (Flood, Fill, Spawn...) rather than dragging.
    pub click_tool: bool,
    /// A tool is selected (Esc deselects).
    pub tool_active: bool,
    /// The tool uses the brush (draw the outline).
    pub brush_tool: bool,
}

const ANNOTATION_COLORS: [[u8; 3]; 16] = [
    [255, 255, 255], [255, 128, 0], [255, 0, 255], [64, 160, 255], [255, 255, 0], [128, 255, 0], [255, 128, 192], [128, 128, 128],
    [192, 192, 192], [0, 192, 192], [160, 32, 240], [0, 0, 255], [139, 69, 19], [0, 128, 0], [255, 0, 0], [0, 0, 0],
];

impl Map2D {
    pub fn new() -> Self {
        Map2D {
            center: (0.0, 0.0),
            zoom: 0,
            cache: FxHashMap::default(),
            hover: None,
            last_paint: None,
            last_time: Instant::now(),
            painting: None,
            wheel_accum: 0.0,
            last_rect: None,
            last_input_frame: u64::MAX,
        }
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn zoom_percent(&self) -> f32 {
        2f32.powi(self.zoom) * 100.0
    }

    /// Screen points per block.
    fn ppb(&self, ppp: f32) -> f32 {
        2f32.powi(self.zoom) / ppp
    }

    /// Change zoom by `steps` powers of two, keeping the world point under `anchor` still.
    pub fn zoom_by(&mut self, steps: i32, rect: Rect, anchor: Pos2, ppp: f32) {
        let new = (self.zoom + steps).clamp(MIN_ZOOM, MAX_ZOOM);
        if new == self.zoom {
            return;
        }
        let before = self.screen_to_world(rect, anchor, ppp);
        self.zoom = new;
        let after = self.screen_to_world(rect, anchor, ppp);
        self.center.0 += (before.0 - after.0) as f64;
        self.center.1 += (before.1 - after.1) as f64;
    }

    /// Keyboard zoom (Ctrl + / - / 0) around the centre of the view.
    pub fn zoom_keyboard(&mut self, steps: i32, ppp: f32) {
        match self.last_rect {
            Some(r) => self.zoom_by(steps, r, r.center(), ppp),
            None => self.zoom = (self.zoom + steps).clamp(MIN_ZOOM, MAX_ZOOM),
        }
    }

    pub fn frame_world(&mut self, d: &Dimension, view: Vec2, ppp: f32) {
        if let Some((x0, z0, x1, z1)) = d.block_bounds() {
            self.center = ((x0 + x1) as f64 / 2.0, (z0 + z1) as f64 / 2.0);
            let (w, h) = ((x1 - x0).max(1) as f32, (z1 - z0).max(1) as f32);
            let mut z = MAX_ZOOM;
            while z > MIN_ZOOM && (w * 2f32.powi(z) / ppp > view.x || h * 2f32.powi(z) / ppp > view.y) {
                z -= 1;
            }
            self.zoom = z.min(2);
        }
    }

    /// Tile textures are rendered at 128 >> lod resolution when zoomed out.
    fn lod(&self) -> u8 {
        (-self.zoom).clamp(0, 4) as u8
    }

    pub fn screen_to_world(&self, rect: Rect, p: Pos2, ppp: f32) -> (f32, f32) {
        let c = rect.center();
        let ppb = self.ppb(ppp);
        ((((p.x - c.x) / ppb) as f64 + self.center.0) as f32, (((p.y - c.y) / ppb) as f64 + self.center.1) as f32)
    }

    pub fn world_to_screen(&self, rect: Rect, x: f32, y: f32, ppp: f32) -> Pos2 {
        let c = rect.center();
        let ppb = self.ppb(ppp);
        // Snap to whole physical pixels so blocks stay crisp.
        let sx = ((c.x as f64 + (x as f64 - self.center.0) * ppb as f64) * ppp as f64).round() / ppp as f64;
        let sy = ((c.y as f64 + (y as f64 - self.center.1) * ppb as f64) * ppp as f64).round() / ppp as f64;
        Pos2::new(sx as f32, sy as f32)
    }

    /// Wheel handling: radius, fine radius, zoom and brush rotation, WorldPainter style.
    fn handle_wheel(&mut self, ui: &egui::Ui, rect: Rect, pointer: Pos2, brush: &mut Brush, ppp: f32) {
        let events: Vec<(egui::MouseWheelUnit, Vec2, egui::Modifiers)> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (unit, delta, m) in events {
            // Shift turns vertical scrolling into horizontal on some platforms.
            let raw = if delta.y != 0.0 { delta.y } else { delta.x };
            let notches = match unit {
                egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => raw.signum() * raw.abs().max(1.0).round(),
                egui::MouseWheelUnit::Point => {
                    self.wheel_accum += raw / 50.0;
                    let n = self.wheel_accum.trunc();
                    self.wheel_accum -= n;
                    n
                }
            };
            if notches == 0.0 {
                continue;
            }
            // Positive = wheel up / away from the user.
            let up = notches > 0.0;
            let n = notches.abs() as i32;
            if m.command || m.ctrl {
                self.zoom_by(if up { n } else { -n }, rect, pointer, ppp);
            } else if m.alt {
                let step = if up { -15.0 } else { 15.0 } * n as f32;
                brush.rotation = (brush.rotation + step + 180.0).rem_euclid(360.0) - 180.0;
            } else if m.shift {
                brush.radius = (brush.radius.round() + if up { -(n as f32) } else { n as f32 }).clamp(0.0, MAX_RADIUS);
            } else {
                let factor = if up { 0.9f32.powi(n) } else { 1.1f32.powi(n) };
                let old = brush.radius.round().max(0.0);
                let mut new = (old * factor).floor();
                if new == old {
                    new = if up { old - 1.0 } else { old + 1.0 };
                }
                if old == 0.0 && !up {
                    new = 1.0;
                }
                brush.radius = new.clamp(0.0, MAX_RADIUS);
            }
        }
    }

    /// Draw the map and handle input. Returns the edit action requested by the user.
    pub fn ui(&mut self, ui: &mut egui::Ui, o: MapUi<'_>) -> MapAction {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.last_rect = Some(rect);
        let ppp = ui.ctx().pixels_per_point();
        let painter = ui.painter_at(rect);
        let bg = Color32::from_rgb(o.style.background[0], o.style.background[1], o.style.background[2]);
        painter.rect_filled(rect, 0.0, bg);

        let pointer = resp.hover_pos();
        let frame = ui.ctx().cumulative_frame_nr();
        let fresh = frame != self.last_input_frame;
        self.last_input_frame = frame;
        if fresh && let Some(p) = pointer {
            self.handle_wheel(ui, rect, p, o.brush, ppp);
        }
        // Panning: middle-drag, or Space + left-drag.
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let panning = resp.dragged_by(egui::PointerButton::Middle) || (space && resp.dragged_by(egui::PointerButton::Primary));
        if panning && fresh {
            let d = resp.drag_delta();
            let ppb = self.ppb(ppp);
            self.center.0 -= (d.x / ppb) as f64;
            self.center.1 -= (d.y / ppb) as f64;
        }

        // Visible tiles
        let lod = self.lod();
        let tl = self.screen_to_world(rect, rect.left_top(), ppp);
        let br = self.screen_to_world(rect, rect.right_bottom(), ppp);
        let (tx0, ty0) = ((tl.0.floor() as i32) >> TILE_SHIFT, (tl.1.floor() as i32) >> TILE_SHIFT);
        let (tx1, ty1) = ((br.0.floor() as i32) >> TILE_SHIFT, (br.1.floor() as i32) >> TILE_SHIFT);
        let style_key = o.style.key();
        let mut missing = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if let Some(t) = o.dim.tile(tx, ty) {
                    let ok = self.cache.get(&(tx, ty, lod)).is_some_and(|c| c.rev == t.rev && c.style == style_key);
                    if !ok {
                        missing.push((tx, ty, t.rev));
                    }
                }
            }
        }
        // Render missing tile images in parallel (bounded per frame to stay responsive).
        let budget = 96;
        let more = missing.len() > budget;
        missing.truncate(budget);
        if !missing.is_empty() {
            let infos = meadow_blocks::state::info_snapshot();
            let images: Vec<((i32, i32, u64), egui::ColorImage)> = missing
                .par_iter()
                .filter_map(|&(tx, ty, rev)| Some(((tx, ty, rev), render_tile(o.world, o.dim, tx, ty, lod, o.style, &infos)?)))
                .collect();
            for ((tx, ty, rev), img) in images {
                let tex = ui.ctx().load_texture(format!("tile{tx},{ty},{lod}"), img, egui::TextureOptions::NEAREST);
                self.cache.insert((tx, ty, lod), CachedTile { rev, style: style_key, tex });
            }
        }
        if more {
            ui.ctx().request_repaint();
        }
        if self.cache.len() > 4000 {
            self.cache.retain(|(x, y, l), _| *l == lod && *x >= tx0 - 4 && *x <= tx1 + 4 && *y >= ty0 - 4 && *y <= ty1 + 4);
        }
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let (ox, oy) = ((tx << TILE_SHIFT) as f32, (ty << TILE_SHIFT) as f32);
                let r = Rect::from_min_max(
                    self.world_to_screen(rect, ox, oy, ppp),
                    self.world_to_screen(rect, ox + TILE_SIZE as f32, oy + TILE_SIZE as f32, ppp),
                );
                if let Some(c) = self.cache.get(&(tx, ty, lod)) {
                    painter.image(c.tex.id(), r, uv, Color32::WHITE);
                }
            }
        }

        if o.show_grid {
            self.draw_grid(&painter, rect, o.grid_size.max(1), ppp);
        }

        // Spawn: small red crosshair.
        let sp = self.world_to_screen(rect, o.world.spawn.0 as f32 + 0.5, o.world.spawn.1 as f32 + 0.5, ppp);
        let red = EStroke::new(1.0, Color32::RED);
        painter.line_segment([sp - Vec2::new(5.0, 0.0), sp + Vec2::new(5.0, 0.0)], red);
        painter.line_segment([sp - Vec2::new(0.0, 5.0), sp + Vec2::new(0.0, 5.0)], red);
        // Centre crosshair (black shadow, white cross).
        let c = rect.center();
        let black = EStroke::new(1.0, Color32::BLACK);
        let white = EStroke::new(1.0, Color32::WHITE);
        painter.line_segment([c + Vec2::new(-4.0, 1.0), c + Vec2::new(6.0, 1.0)], black);
        painter.line_segment([c + Vec2::new(1.0, -4.0), c + Vec2::new(1.0, 6.0)], black);
        painter.line_segment([c - Vec2::new(5.0, 0.0), c + Vec2::new(5.0, 0.0)], white);
        painter.line_segment([c - Vec2::new(0.0, 5.0), c + Vec2::new(0.0, 5.0)], white);

        // Brush outline: black line with a white dashed line on top, in the brush's shape.
        self.hover = None;
        if let Some(p) = pointer {
            let (wx, wy) = self.screen_to_world(rect, p, ppp);
            self.hover = Some((wx.floor() as i32, wy.floor() as i32));
            if o.tool_active && o.brush_tool {
                let r = (o.brush.radius.max(0.5) * self.ppb(ppp)).max(1.0);
                let pts: Vec<Pos2> = match o.brush.shape {
                    BrushShape::Circle => (0..=96).map(|k| {
                        let a = k as f32 / 96.0 * std::f32::consts::TAU;
                        p + Vec2::new(a.cos(), a.sin()) * r
                    }).collect(),
                    BrushShape::Square => {
                        let (s, cs) = o.brush.rotation.to_radians().sin_cos();
                        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0)]
                            .iter()
                            .map(|(x, y)| p + Vec2::new(x * cs - y * s, x * s + y * cs) * r)
                            .collect()
                    }
                };
                painter.add(Shape::line(pts.clone(), black));
                painter.extend(Shape::dashed_line(&pts, white, 4.0, 6.0));
            }
        }

        // Editing
        if !fresh {
            return MapAction::None;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_time).as_secs_f32().clamp(0.001, 0.1);
        self.last_time = now;
        if panning || space || !o.tool_active {
            if self.painting.take().is_some() {
                self.last_paint = None;
                return MapAction::EndPaint;
            }
            return MapAction::None;
        }
        let (primary, secondary, alt) = ui.input(|i| (i.pointer.primary_down(), i.pointer.secondary_down(), i.modifiers.alt));
        if o.click_tool {
            if let Some((x, y)) = self.hover {
                if resp.clicked() {
                    return MapAction::Click { x, y, inverse: alt };
                }
                if resp.secondary_clicked() {
                    return MapAction::Click { x, y, inverse: true };
                }
            }
            return MapAction::None;
        }
        let held = primary || secondary;
        if held
            && (self.painting.is_some() || resp.is_pointer_button_down_on())
            && let Some(p) = pointer.or(ui.input(|i| i.pointer.interact_pos()))
        {
            let inverse = *self.painting.get_or_insert(secondary || alt);
            let w = self.screen_to_world(rect, p, ppp);
            let first = self.last_paint.is_none();
            let from = self.last_paint.unwrap_or(w);
            self.last_paint = Some(w);
            ui.ctx().request_repaint();
            return MapAction::Paint { from, to: w, dt, first, inverse };
        }
        if self.painting.is_some() && !held {
            self.painting = None;
            self.last_paint = None;
            return MapAction::EndPaint;
        }
        MapAction::None
    }

    /// WorldPainter-style grid: black lines, coordinate labels along the top and left edges,
    /// labels in bold on region (512-block) borders.
    fn draw_grid(&self, painter: &egui::Painter, rect: Rect, grid: i32, ppp: f32) {
        let ppb = self.ppb(ppp);
        if grid as f32 * ppb < 4.0 {
            return;
        }
        let tl = self.screen_to_world(rect, rect.left_top(), ppp);
        let br = self.screen_to_world(rect, rect.right_bottom(), ppp);
        let (gx0, gy0) = ((tl.0 / grid as f32).floor() as i32, (tl.1 / grid as f32).floor() as i32);
        let (gx1, gy1) = ((br.0 / grid as f32).ceil() as i32, (br.1 / grid as f32).ceil() as i32);
        let line = EStroke::new(1.0, Color32::BLACK);
        // Label every n-th line so labels never overlap.
        let every = ((52.0 / (grid as f32 * ppb)).ceil() as i32).max(1);
        let font = FontId::proportional(10.0);
        let region = |v: i32| (v as i64 * grid as i64) % 512 == 0;
        for gx in gx0..=gx1 {
            let x = self.world_to_screen(rect, (gx * grid) as f32, 0.0, ppp).x + 0.5 / ppp;
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], line);
            if gx.rem_euclid(every) == 0 {
                label(painter, Pos2::new(x + 2.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, &(gx * grid).to_string(), &font, region(gx));
            }
        }
        for gy in gy0..=gy1 {
            let y = self.world_to_screen(rect, 0.0, (gy * grid) as f32, ppp).y + 0.5 / ppp;
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], line);
            if gy.rem_euclid(every) == 0 {
                label(painter, Pos2::new(rect.left() + 2.0, y - 2.0), egui::Align2::LEFT_BOTTOM, &(gy * grid).to_string(), &font, region(gy));
            }
        }
    }
}

fn label(painter: &egui::Painter, pos: Pos2, align: egui::Align2, text: &str, font: &FontId, bold: bool) {
    painter.text(pos, align, text, font.clone(), Color32::BLACK);
    if bold {
        painter.text(pos + Vec2::new(0.6, 0.0), align, text, font.clone(), Color32::BLACK);
    }
}

// ---------------------------------------------------------------------------------------------
// Tile rendering (WorldPainter's TileRenderer rules)

#[inline]
fn mix(c1: [i32; 3], c2: [i32; 3], alpha: i32) -> [i32; 3] {
    [0, 1, 2].map(|k| (c1[k] * alpha + c2[k] * (256 - alpha)) / 256)
}

#[inline]
fn multiply(c: [i32; 3], amount: i32) -> [i32; 3] {
    c.map(|v| (v * amount / 256).min(255))
}

fn hex(c: u32) -> [i32; 3] {
    [((c >> 16) & 255) as i32, ((c >> 8) & 255) as i32, (c & 255) as i32]
}

type Pattern = [u8; 8];

// 8x8 stamps, one byte per row, MSB = left-most pixel (WorldPainter's forest patterns).
const DECIDUOUS: Pattern = [0b00111000, 0b01111100, 0b11111110, 0b11111110, 0b01111100, 0b00111000, 0b00010000, 0b00000000];
const PINE: Pattern = [0b00010000, 0b00111000, 0b00111000, 0b01111100, 0b01111100, 0b11111110, 0b00010000, 0b00000000];
const JUNGLE: Pattern = [0b11111000, 0b11111001, 0b11111001, 0b00111000, 0b00111110, 0b00111111, 0b00111111, 0b00111000];
const SWAMP: Pattern = [0b00000000, 0b00011000, 0b11011011, 0b01111110, 0b00111100, 0b00000000, 0b11111110, 0b11111110];

fn tree_style(kind: TreeKind) -> (u32, &'static Pattern) {
    match kind {
        TreeKind::Deciduous => (0x004000, &DECIDUOUS),
        TreeKind::Pine => (0x002000, &PINE),
        TreeKind::Jungle => (0x003000, &JUNGLE),
        TreeKind::Swamp => (0x002000, &SWAMP),
        TreeKind::Mangrove => (0x203010, &SWAMP),
        TreeKind::Cherry => (0x9a4870, &DECIDUOUS),
        TreeKind::PaleOak => (0x505850, &DECIDUOUS),
    }
}

#[inline]
fn pattern_hit(p: &Pattern, x: i32, y: i32) -> bool {
    (p[(y & 7) as usize] >> (7 - (x & 7))) & 1 == 1
}

/// Render one tile to an image at 128 >> lod resolution.
pub fn render_tile(world: &World, dim: &Dimension, tx: i32, ty: i32, lod: u8, style: &MapStyle, infos: &[StateInfo]) -> Option<egui::ColorImage> {
    let tile = dim.tile(tx, ty)?;
    let step = 1i32 << lod;
    let n = (TILE_SIZE / step) as usize;
    let (ox, oy) = (tx << TILE_SHIFT, ty << TILE_SHIFT);
    let layers: Vec<&LayerDef> = style.layers.iter().filter_map(|id| world.layer(*id)).collect();
    let color_of = |s: meadow_blocks::StateId| -> [i32; 3] {
        let c = infos.get(s.0 as usize).map(|i| i.color).unwrap_or_else(|| s.info().color);
        [c[0] as i32, c[1] as i32, c[2] as i32]
    };
    let water_c = color_of(meadow_blocks::common::water());
    let lava_c = color_of(meadow_blocks::common::lava());
    let bedrock_c = color_of(meadow_blocks::common::bedrock());
    let bg = style.background.map(|v| v as i32);
    let min_y = dim.limits.min_y;
    let seed = world.seed;
    // Integer heights; neighbours across the tile edge come from the dimension.
    let ih = |lx: i32, ly: i32| -> i32 {
        if (0..TILE_SIZE).contains(&lx) && (0..TILE_SIZE).contains(&ly) {
            (tile.height[local_index(lx, ly)] + 0.5).floor() as i32
        } else {
            dim.int_height(ox + lx, oy + ly).unwrap_or_else(|| (tile.height[local_index(lx.clamp(0, TILE_SIZE - 1), ly.clamp(0, TILE_SIZE - 1))] + 0.5).floor() as i32)
        }
    };
    // Fluid level of a neighbour if it is flooded, else `own`.
    let fluid = |lx: i32, ly: i32, own: i32| -> i32 {
        let (h, w) = if (0..TILE_SIZE).contains(&lx) && (0..TILE_SIZE).contains(&ly) {
            let i = local_index(lx, ly);
            ((tile.height[i] + 0.5).floor() as i32, tile.water[i] as i32)
        } else {
            match (dim.int_height(ox + lx, oy + ly), dim.water(ox + lx, oy + ly)) {
                (Some(h), Some(w)) => (h, w),
                _ => return own,
            }
        };
        if w > h { w } else { own }
    };
    let mut px = vec![Color32::BLACK; n * n];
    for py in 0..n {
        for pxx in 0..n {
            let (lx, ly) = (pxx as i32 * step, py as i32 * step);
            let (wx, wy) = (ox + lx, oy + ly);
            let i = local_index(lx, ly);
            if tile.layer_value(LayerId::VOID, i, 0) != 0 {
                px[py * n + pxx] = Color32::from_rgb(bg[0] as u8, bg[1] as u8, bg[2] as u8);
                continue;
            }
            let h = ih(lx, ly);
            let (d_e, d_w, d_s, d_n) = (ih(lx + 1, ly) - h, ih(lx - 1, ly) - h, ih(lx, ly + 1) - h, ih(lx, ly - 1) - h);
            let water = tile.water[i] as i32;
            let flooded = water > h;
            let mut c = if style.contours && style.contour_spacing > 0 && h % style.contour_spacing == 0 && (d_e < 0 || d_w < 0 || d_s < 0 || d_n < 0) {
                [0, 0, 0]
            } else {
                let base = if flooded {
                    if tile.layer_value(LayerId::LAVA, i, 0) != 0 { lava_c } else { water_c }
                } else if h <= min_y {
                    bedrock_c
                } else {
                    let terrain = world.terrain(tile.terrain[i]);
                    match terrain.block_at(seed, wx, wy, h, 0) {
                        Some(b) if infos.get(b.0 as usize).is_none_or(|inf| inf.color[3] > 0) => color_of(b),
                        _ => terrain.color.map(|v| v as i32),
                    }
                };
                let mut c = base;
                for def in &layers {
                    let v = tile.layer_value(def.id, i, def.default);
                    if v == def.default {
                        continue;
                    }
                    let alpha = |max: u8| (v as i32 * 256 / max.max(1) as i32).min(256);
                    c = match &def.kind {
                        LayerKind::Frost => c.map(|v| 255 - (255 - v) / 2),
                        LayerKind::Trees(kind) => {
                            let (col, pat) = tree_style(*kind);
                            if pattern_hit(pat, wx, wy) { mix(hex(col), c, alpha(15)) } else { c }
                        }
                        LayerKind::Caves => mix(hex(0xC000FF), c, alpha(15)),
                        LayerKind::Caverns => mix(hex(0xFF00FF), c, alpha(15)),
                        LayerKind::Chasms => mix(hex(0xC000FF), c, alpha(15)),
                        LayerKind::Resources => mix(hex(0x0000FF), c, alpha(15)),
                        LayerKind::Annotations => mix(ANNOTATION_COLORS[(v & 15) as usize].map(|x| x as i32), c, 192),
                        LayerKind::Biome => {
                            let b = meadow_blocks::biomes().by_numeric(v).map(|b| b.color).unwrap_or([255, 0, 255]);
                            mix(b.map(|x| x as i32), c, 128)
                        }
                        LayerKind::PaintedMask => mix([255, 60, 60], c, v as i32 * 160 / 255),
                        LayerKind::FloodWithLava | LayerKind::Void | LayerKind::Populate | LayerKind::ReadOnly => c,
                        _ => {
                            let col = def.color.map(|x| x as i32);
                            if def.is_bits() { mix(col, c, 128) } else { mix(col, c, alpha(def.data.max()) / 2 + 32) }
                        }
                    };
                }
                if let Some(m) = &style.mask {
                    let v = m.eval(dim, wx, wy);
                    if v > 0.0 {
                        c = mix(m.color.map(|x| x as i32), c, (v * 115.0) as i32);
                    }
                }
                c
            };
            c = multiply(c, style.light.brighten(d_e, d_w, d_s, d_n, style.hillshade));
            if flooded {
                let (f_e, f_w, f_s, f_n) = (
                    fluid(lx + 1, ly, water) - water,
                    fluid(lx - 1, ly, water) - water,
                    fluid(lx, ly + 1, water) - water,
                    fluid(lx, ly - 1, water) - water,
                );
                c = multiply(c, style.light.brighten(f_e, f_w, f_s, f_n, style.hillshade));
            }
            px[py * n + pxx] = Color32::from_rgb(c[0].clamp(0, 255) as u8, c[1].clamp(0, 255) as u8, c[2].clamp(0, 255) as u8);
        }
    }
    Some(egui::ColorImage::new([n, n], px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_cycle_and_shading() {
        let mut l = LightOrigin::NorthWest;
        for _ in 0..5 {
            l = l.right();
        }
        assert_eq!(l, LightOrigin::NorthWest);
        assert_eq!(LightOrigin::NorthWest.left(), LightOrigin::Above);
        // A slope rising to the east is lit from the north-west: +12.5 % per block.
        assert_eq!(LightOrigin::NorthWest.brighten(1, -1, 0, 0, 1.0), 256 + 64);
        assert_eq!(LightOrigin::Above.brighten(5, -5, 5, -5, 1.0), 256);
        assert_eq!(multiply([200, 100, 50], 320), [250, 125, 62]);
        assert_eq!(mix([0, 64, 0], [200, 200, 200], 256), [0, 64, 0]);
        assert!(pattern_hit(&DECIDUOUS, 3, 0) && !pattern_hit(&DECIDUOUS, 0, 0));
    }

    #[test]
    fn worldpainter_rendering_rules() {
        use meadow_core::world::NewWorld;
        let mut w = World::new(&NewWorld { width_tiles: 1, height_tiles: 1, base_height: 70.0, water_level: 62, ..Default::default() });
        // Lake, terrace (for contours), frost
        let d = w.surface_mut();
        for y in 0..20 {
            for x in 0..20 {
                d.set_height(x, y, 50.0);
            }
        }
        for y in 40..60 {
            d.set_height(80, y, 80.0);
        }
        d.set_layer_value(LayerId::FROST, true, 0, 100, 100, 1);
        let style = MapStyle {
            light: LightOrigin::NorthWest,
            hillshade: 1.0,
            contours: true,
            contour_spacing: 10,
            layers: vec![LayerId::FROST],
            mask: None,
            background: [0xC0, 0xFF, 0xFF],
            defs_gen: 0,
            colors_gen: 0,
        };
        let infos = meadow_blocks::state::info_snapshot();
        let img = render_tile(&w, w.surface(), 0, 0, 0, &style, &infos).unwrap();
        let at = |x: usize, y: usize| img.pixels[y * 128 + x];
        // Flat water: lake interior pixels identical (no depth fade).
        assert_eq!(at(5, 5), at(10, 10));
        let water = meadow_blocks::common::water().info().color;
        assert_eq!(at(5, 5), Color32::from_rgb(water[0], water[1], water[2]));
        // Contour: column at height 80 with lower neighbours is black.
        assert_eq!(at(80, 50), Color32::BLACK);
        // Frost lightens the ground halfway to white.
        let g = at(110, 110);
        let f = at(100, 100);
        assert_eq!(f.r() as i32, 255 - (255 - g.r() as i32) / 2);
    }
}
