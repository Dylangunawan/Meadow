//! Orbit/fly camera with a reverse-Z infinite perspective projection.

use glam::{Mat4, Vec3, Vec4};

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// Point the orbit camera looks at.
    pub target: Vec3,
    /// Distance from the target.
    pub distance: f32,
    /// Radians; 0 looks towards -Z (north).
    pub yaw: f32,
    /// Radians; positive looks down.
    pub pitch: f32,
    pub fov_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { target: Vec3::new(0.0, 64.0, 0.0), distance: 300.0, yaw: 0.6, pitch: 0.6, fov_y: 55f32.to_radians() }
    }
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, -sp, -cy * cp).normalize()
    }
    pub fn eye(&self) -> Vec3 {
        self.target - self.forward() * self.distance
    }
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }
    pub fn proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_infinite_reverse_rh(self.fov_y, aspect.max(0.01), 0.25)
    }
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.45, 1.55);
    }
    pub fn zoom(&mut self, steps: f32) {
        self.distance = (self.distance * (1.0 - steps * 0.1)).clamp(4.0, 20000.0);
    }
    /// Pan in the view plane (screen pixels scaled by distance).
    pub fn pan(&mut self, dx: f32, dy: f32) {
        let f = self.forward();
        let right = f.cross(Vec3::Y).normalize_or_zero();
        let flat_fwd = Vec3::new(f.x, 0.0, f.z).normalize_or_zero();
        let k = self.distance * 0.0015;
        self.target += -right * dx * k + flat_fwd * dy * k;
    }
    /// Fly movement relative to the view direction (blocks).
    pub fn fly(&mut self, forward: f32, right: f32, up: f32) {
        let f = self.forward();
        let r = f.cross(Vec3::Y).normalize_or_zero();
        self.target += f * forward + r * right + Vec3::Y * up;
    }

    /// Ray through a point in normalised device coordinates.
    pub fn ray(&self, ndc_x: f32, ndc_y: f32, aspect: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(aspect).inverse();
        // Reverse Z: near plane at depth 1, far towards 0.
        let near = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        let far = inv * Vec4::new(ndc_x, ndc_y, 0.01, 1.0);
        let near = near.truncate() / near.w;
        let far = far.truncate() / far.w;
        (near, (far - near).normalize())
    }
}

/// View frustum planes for culling (inward-facing).
pub struct Frustum {
    planes: [Vec4; 5],
}

impl Frustum {
    pub fn from_view_proj(m: Mat4) -> Frustum {
        let r = |i: usize| m.row(i);
        // Reverse-Z infinite projection: skip the far plane.
        let planes = [r(3) + r(0), r(3) - r(0), r(3) + r(1), r(3) - r(1), r(3) - r(2)];
        let planes = planes.map(|p| p / p.truncate().length());
        Frustum { planes }
    }
    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        for p in &self.planes {
            let v = Vec3::new(if p.x >= 0.0 { max.x } else { min.x }, if p.y >= 0.0 { max.y } else { min.y }, if p.z >= 0.0 { max.z } else { min.z });
            if p.truncate().dot(v) + p.w < 0.0 {
                return false;
            }
        }
        true
    }
}
