//! Real-time 3D rendering for Meadow: a GPU viewport of the whole world (with LOD, culling and
//! background meshing) and the custom-object placement preview. Works inside the egui app and
//! headless (for screenshots and tests).

pub mod camera;
pub mod mesh;
pub mod preview;
pub mod scene;

pub use camera::Camera;
pub use egui_wgpu::wgpu;
pub use scene::{COLOR_FORMAT, Renderer, Scene, SceneSettings, SceneStats};

#[cfg(test)]
mod tests;
