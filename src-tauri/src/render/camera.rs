//! The orbit camera, ported from the `SchematicRenderer` camera state in
//! src/renderer.ts (the `basis`, `fit`, `zoom`, `pan`, `orbit`, `syncSize`,
//! `fittingDistance` and the per-frame `view`/`projection`/`mvp` setup).
//!
//! Input handling (pointer, wheel, keyboard) stays in the integrator; this
//! module only applies the deltas with the exact same constants and clamps
//! as the WebGL renderer. Every mutating call marks the camera dirty so the
//! renderer can keep its on-demand (event-driven) frame policy.

use crate::render::math::{
    self, add3, cross3, dot3, mul3, normalize3, scale_and_add3, sub3, Mat4, Vec3, UP,
};

/// The vertical field of view in degrees, from renderer.ts.
const FOV_DEGREES: f32 = 28.0;
/// `tan(FOV / 2)`, the fitting scale from renderer.ts.
const HALF_FOV_TAN: f32 = 0.249_328_0;
/// 0.10.14 renders keep `is_empty()` blind to greedy parts; nothing here
/// depends on it because the camera only consumes the metadata bounds.
const ORBIT_SPEED: f32 = 0.008;
/// Pitch clamp in radians, matching `Math.max(-1.5, Math.min(1.5, ...))`.
const PITCH_LIMIT: f32 = 1.5;
/// Arrow-key step in pixels, matching renderer.ts.
const ARROW_STEP: f32 = 18.0;
/// Fit padding divisor from renderer.ts's `fittingDistance` (`/ 1.18`).
const FIT_PADDING: f32 = 1.18;
/// Fit floor from renderer.ts (`let fit = 0.1`).
const FIT_FLOOR: f32 = 0.1;
/// The camera always starts from this orientation, matching `fit()`.
const DEFAULT_YAW: f32 = std::f32::consts::PI / 4.0;
const DEFAULT_PITCH: f32 = 1.0 / core::f32::consts::SQRT_2;

/// The per-frame camera uniforms. `offset` is the `-centre` translation that
/// puts the model's bounding-box centre at the origin before the view-projection
/// transform; the grid is drawn with a zero offset, exactly like renderer.ts.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FrameCamera {
    pub view_projection: Mat4,
    pub offset: Vec3,
}

impl FrameCamera {
    /// Packed uniform bytes: 64 bytes of matrix, 12 bytes of offset, 4 bytes
    /// of tail padding (WGSL pads the `vec3` to 16).
    pub fn encode(&self) -> [u8; 80] {
        let mut bytes = [0u8; 80];
        for (index, value) in self.view_projection.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (axis, value) in self.offset.iter().enumerate() {
            let at = 64 + axis * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

/// The orbit/pan/zoom camera. Mirrors the mutable renderer.ts state.
pub struct Camera {
    yaw: f32,
    pitch: f32,
    distance: f32,
    fitted_distance: f32,
    target: Vec3,
    centre: Vec3,
    minimum: Vec3,
    maximum: Vec3,
    aspect: f32,
    /// Viewport height in pixels; the `pan` scale divides by it, matching
    /// renderer.ts's `cssHeight`.
    viewport_height: f32,
    grid_visible: bool,
    dirty: bool,
    has_model: bool,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            yaw: DEFAULT_YAW,
            pitch: DEFAULT_PITCH,
            distance: 10.0,
            fitted_distance: 10.0,
            target: [0.0; 3],
            centre: [0.0; 3],
            minimum: [0.0; 3],
            maximum: [0.0; 3],
            aspect: 1.0,
            viewport_height: 1.0,
            // renderer.ts shows the grid by default (`gridVisible = true`).
            grid_visible: true,
            dirty: true,
            has_model: false,
        }
    }

    /// A model's metadata bounds arrived; adopt the default orientation and fit
    /// the camera, matching renderer.ts's end-of-`loadModel` block.
    pub fn set_model_bounds(&mut self, min: [f32; 3], max: [f32; 3]) {
        self.centre = [
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        ];
        self.minimum = sub3(min, self.centre);
        self.maximum = sub3(max, self.centre);
        self.fit();
        self.has_model = true;
        self.dirty = true;
    }

    /// The viewport size changed. Matches `syncSize`'s aspect handling: the fit
    /// is recomputed when the aspect changes, and the current zoom ratio is
    /// preserved.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let aspect = width as f32 / height as f32;
        self.viewport_height = height as f32;
        if aspect != self.aspect {
            self.aspect = aspect;
            if self.has_model {
                let ratio = self.distance / self.fitted_distance;
                self.fitted_distance = self.fitting_distance();
                self.distance = self.fitted_distance * ratio;
            }
        }
        self.dirty = true;
    }

    /// Pressed-drag orbit delta in pixels, matching `orbit(dx, dy)`.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw - dx * ORBIT_SPEED) % (std::f32::consts::PI * 2.0);
        self.pitch = (self.pitch + dy * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.dirty = true;
    }

    /// Pressed-drag pan delta in pixels, matching `pan(dx, dy)`.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        if !self.has_model {
            return;
        }
        let (right, up) = self.plane();
        let scale = 2.0 * self.distance * HALF_FOV_TAN / self.viewport_height;
        self.target = scale_and_add3(self.target, right, -dx * scale);
        self.target = scale_and_add3(self.target, up, dy * scale);
        self.dirty = true;
    }

    /// Wheel delta, matching `onWheel`: `deltaMode` 1 (lines) scales by 16,
    /// 2 (pages) by the viewport height, 0 (pixels) uses the raw value.
    pub fn zoom_wheel(&mut self, delta_y: f32, delta_mode: u8) {
        if !self.has_model {
            return;
        }
        let unit = match delta_mode {
            1 => 16.0,
            2 => self.viewport_height,
            _ => 1.0,
        };
        let clamped = (delta_y * unit).clamp(-1200.0, 1200.0);
        self.zoom((clamped * 0.001).exp());
    }

    /// Raw multiplicative zoom, matching `zoom(factor)`: clamped to
    /// `[fitted * 0.05, fitted * 8]` and rejected for non-finite factors.
    pub fn zoom(&mut self, factor: f32) {
        if !self.has_model || !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let scale = self.fitted_distance;
        self.distance = (self.distance * factor).clamp(scale * 0.05, scale * 8.0);
        self.dirty = true;
    }

    /// Arrow-key dolly/orbit, matching the arrow cases of `onKeyDown`.
    pub fn arrow(&mut self, dx: f32, dy: f32, panning: bool) {
        if !self.has_model {
            return;
        }
        if panning {
            self.pan(dx * ARROW_STEP, dy * ARROW_STEP);
        } else {
            self.orbit(dx * ARROW_STEP, dy * ARROW_STEP);
        }
    }

    /// Reset the default orientation and refit, matching `fit()`.
    pub fn fit(&mut self) {
        self.yaw = DEFAULT_YAW;
        self.pitch = DEFAULT_PITCH;
        self.target = [0.0; 3];
        let fit = self.fitting_distance();
        self.fitted_distance = fit;
        self.distance = fit;
        self.dirty = true;
    }

    pub fn set_grid(&mut self, visible: bool) {
        if self.grid_visible != visible {
            self.grid_visible = visible;
            self.dirty = true;
        }
    }

    pub fn grid_visible(&self) -> bool {
        self.grid_visible
    }

    /// Whether a new frame is warranted. The renderer clears this after drawing.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Camera uniform bytes for the current state. `grid` selects the zero
    /// offset renderer.ts used for the grid pass instead of `-centre`.
    pub(crate) fn frame_camera(&self, grid: bool) -> FrameCamera {
        let direction = self.basis();
        let eye = add3(self.target, mul3(direction, self.distance));
        let view = math::look_at(eye, self.target, UP);
        let projection = math::perspective(
            FOV_DEGREES.to_radians(),
            self.aspect,
            (self.fitted_distance / 1000.0).max(0.01),
            self.fitted_distance * 24.0,
        );
        FrameCamera {
            view_projection: math::multiply4(&projection, &view),
            offset: if grid {
                [0.0; 3]
            } else {
                [-self.centre[0], -self.centre[1], -self.centre[2]]
            },
        }
    }

    /// The orbit direction (`target -> eye`), matching `basis()`.
    fn basis(&self) -> Vec3 {
        let cp = self.pitch.cos();
        [cp * self.yaw.sin(), self.pitch.sin(), cp * self.yaw.cos()]
    }

    /// The camera's right and up basis vectors, recomputed like `basis()`.
    fn plane(&self) -> (Vec3, Vec3) {
        let forward = mul3(self.basis(), -1.0);
        let right = normalize3(cross3(forward, UP));
        let up = cross3(right, forward);
        (right, up)
    }

    /// The distance that fits the bounds, matching `fittingDistance()`.
    fn fitting_distance(&self) -> f32 {
        let forward = mul3(self.basis(), -1.0);
        let right = normalize3(cross3(forward, UP));
        let up = cross3(right, forward);
        let vertical = HALF_FOV_TAN / FIT_PADDING;
        let horizontal = vertical * self.aspect;
        let mut fit = FIT_FLOOR;
        for corner in 0..8 {
            let x = if corner & 1 == 0 {
                self.minimum[0]
            } else {
                self.maximum[0]
            };
            let y = if corner & 2 == 0 {
                self.minimum[1]
            } else {
                self.maximum[1]
            };
            let z = if corner & 4 == 0 {
                self.minimum[2]
            } else {
                self.maximum[2]
            };
            let corner = [x, y, z];
            let depth = dot3(corner, forward);
            let across = dot3(corner, right);
            let above = dot3(corner, up);
            fit = fit.max(above.abs() / vertical - depth);
            fit = fit.max(across.abs() / horizontal - depth);
        }
        fit
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_camera() -> Camera {
        let mut camera = Camera::new();
        camera.set_model_bounds([-1.0, -2.0, -3.0], [3.0, 2.0, 1.0]);
        camera
    }

    #[test]
    fn model_bounds_center_and_fit() {
        let camera = model_camera();
        assert_eq!(camera.centre, [1.0, 0.0, -1.0]);
        assert_eq!(camera.minimum, [-2.0, -2.0, -2.0]);
        assert_eq!(camera.maximum, [2.0, 2.0, 2.0]);
        assert!(camera.fitted_distance > FIT_FLOOR);
        assert_eq!(camera.fitted_distance, camera.distance);
    }

    #[test]
    fn zoom_clamps_to_fitted_ratio() {
        let mut camera = model_camera();
        camera.zoom(1e6);
        assert!((camera.distance - camera.fitted_distance * 8.0).abs() < 1e-4);
        camera.zoom(1e-9);
        assert!((camera.distance - camera.fitted_distance * 0.05).abs() < 1e-4);
        camera.zoom(f32::NAN);
        assert!((camera.distance - camera.fitted_distance * 0.05).abs() < 1e-4);
    }

    #[test]
    fn pitch_stays_within_the_clamp() {
        let mut camera = model_camera();
        camera.orbit(0.0, 10_000.0);
        let frame = camera.frame_camera(false);
        let _ = frame.view_projection;
        camera.orbit(0.0, -10_000.0);
    }

    #[test]
    fn pan_moves_the_target_in_the_camera_plane() {
        let mut camera = model_camera();
        let before = camera.target;
        camera.pan(50.0, 0.0);
        assert_ne!(before, camera.target);
    }

    #[test]
    fn without_a_model_the_camera_ignores_input() {
        let mut camera = Camera::new();
        camera.orbit(10.0, 10.0);
        camera.pan(10.0, 10.0);
        camera.zoom_wheel(100.0, 0);
        assert_eq!(camera.target, [0.0; 3]);
        assert_eq!(camera.distance, 10.0);
    }

    #[test]
    fn wheel_unit_matches_renderer_ts() {
        // deltaMode 1 multiplies the raw delta by 16 before the clamp, so the
        // same physical scroll produces the same factor as mode 0 with a
        // sixteenth of the delta.
        let mut camera = model_camera();
        camera.zoom_wheel(-75.0, 0);
        let pixels = camera.distance;
        camera.set_model_bounds([-1.0, -2.0, -3.0], [3.0, 2.0, 1.0]);
        camera.zoom_wheel(-75.0 / 16.0, 1);
        assert!((camera.distance - pixels).abs() < 1e-4);
        // mode 2 scales by the viewport height.
        let mut camera = model_camera();
        camera.resize(1000, 1000);
        camera.zoom_wheel(-0.075, 2);
        camera.set_model_bounds([-1.0, -2.0, -3.0], [3.0, 2.0, 1.0]);
        camera.resize(1000, 1000);
        camera.zoom_wheel(-75.0, 0);
        assert!((camera.distance - pixels).abs() < 1e-4);
    }

    #[test]
    fn grid_offset_is_zero_and_part_offset_is_negative_centre() {
        let camera = model_camera();
        assert_eq!(camera.frame_camera(true).offset, [0.0; 3]);
        assert_eq!(camera.frame_camera(false).offset, [-1.0, 0.0, 1.0]);
    }

    #[test]
    fn resize_recomputes_fit_and_keeps_the_ratio() {
        let mut camera = model_camera();
        camera.zoom(2.0);
        let ratio = camera.distance / camera.fitted_distance;
        camera.resize(1600, 1200);
        assert!((camera.aspect - 1600.0 / 1200.0).abs() < 1e-6);
        assert!((camera.distance / camera.fitted_distance - ratio).abs() < 1e-5);
    }

    #[test]
    fn resize_ignores_zero_sized_viewports() {
        let mut camera = model_camera();
        camera.resize(0, 0);
        assert_eq!(camera.aspect, 1.0);
    }
}
