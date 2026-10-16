//! Column-major vector and matrix helpers uploaded directly to WGSL
//! `mat4x4<f32>`.
//!
//! Perspective matrices use wgpu clip space: `z` is in `[0, 1]`, with near
//! mapping to 0 and far to 1.

/// A 3-component vector.
pub type Vec3 = [f32; 3];

/// A 4x4 matrix in column-major order.
pub type Mat4 = [f32; 16];

/// World-up direction for camera orbit.
pub const UP: Vec3 = [0.0, 1.0, 0.0];

pub fn dot3(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross3(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn normalize3(v: Vec3) -> Vec3 {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 0.0 {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        [0.0, 0.0, 0.0]
    }
}

pub fn scale_and_add3(out: Vec3, direction: Vec3, scale: f32) -> Vec3 {
    [
        out[0] + direction[0] * scale,
        out[1] + direction[1] * scale,
        out[2] + direction[2] * scale,
    ]
}

pub fn sub3(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add3(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn mul3(v: Vec3, scale: f32) -> Vec3 {
    [v[0] * scale, v[1] * scale, v[2] * scale]
}

/// Right-handed perspective matrix; `near` maps to 0 and `far` to 1.
/// `fov_y_radians` is the vertical field of view; `aspect` is width over height.
pub fn perspective(fov_y_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y_radians / 2.0).tan();
    let mut m = [0.0f32; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = far / (near - far);
    m[11] = -1.0;
    m[14] = near * far / (near - far);
    m
}

/// Right-handed look-at matrix; the camera looks down negative Z towards `centre`.
pub fn look_at(eye: Vec3, centre: Vec3, up: Vec3) -> Mat4 {
    let z = normalize3(sub3(eye, centre));
    let x = normalize3(cross3(up, z));
    let y = cross3(z, x);
    [
        x[0],
        y[0],
        z[0],
        0.0, //
        x[1],
        y[1],
        z[1],
        0.0, //
        x[2],
        y[2],
        z[2],
        0.0,           //
        -dot3(x, eye), //
        -dot3(y, eye), //
        -dot3(z, eye), //
        1.0,
    ]
}

/// Returns `a * b`, applying `b` before `a`.
pub fn multiply4(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0f32; 16];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[k * 4 + row] * b[column * 4 + k];
            }
            out[column * 4 + row] = sum;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform(matrix: &Mat4, point: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|row| {
            (0..4)
                .map(|column| matrix[column * 4 + row] * point[column])
                .sum()
        })
    }

    #[test]
    fn perspective_maps_near_and_far_to_zero_and_one() {
        let m = perspective(28.0f32.to_radians(), 1.5, 0.01, 0.24);
        let near = [0.0, 0.0, -0.01, 1.0];
        let far = [0.0, 0.0, -0.24, 1.0];
        let (nc, fc) = (transform(&m, near), transform(&m, far));
        assert!(nc[3] > 0.0 && fc[3] > 0.0);
        assert!((nc[2] / nc[3]).abs() < 1e-6, "near must map to z = 0");
        assert!(
            ((fc[2] / fc[3]) - 1.0).abs() < 1e-4,
            "far must map to z = 1"
        );
    }

    #[test]
    fn multiply_applies_the_right_hand_transform_first() {
        let scale: Mat4 = [
            2.0, 0.0, 0.0, 0.0, //
            0.0, 3.0, 0.0, 0.0, //
            0.0, 0.0, 4.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let translation: Mat4 = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            5.0, 6.0, 7.0, 1.0,
        ];
        let point = [1.0, 2.0, 3.0, 1.0];
        assert_eq!(
            transform(&multiply4(&translation, &scale), point),
            [7.0, 12.0, 19.0, 1.0]
        );
        assert_eq!(
            transform(&multiply4(&scale, &translation), point),
            [12.0, 24.0, 40.0, 1.0]
        );
    }

    #[test]
    fn look_at_transforms_an_arbitrary_eye_to_the_origin() {
        let eye = [-3.0, 4.0, 9.0];
        let view = look_at(eye, [2.0, -1.0, 5.0], UP);
        let camera_origin = transform(&view, [eye[0], eye[1], eye[2], 1.0]);
        for coordinate in &camera_origin[..3] {
            assert!(
                coordinate.abs() < 1e-5,
                "eye must map to the origin: {camera_origin:?}"
            );
        }
        assert_eq!(camera_origin[3], 1.0);
    }

    #[test]
    fn look_at_centres_an_off_axis_target_throughout_an_orbit() {
        let target = [3.0, -2.0, 5.0];
        let projection = perspective(28.0f32.to_radians(), 1.5, 0.1, 100.0);
        for (yaw, pitch) in [(0.0f32, 0.0f32), (0.7, 0.6), (-1.2, -0.4), (2.4, 1.2)] {
            let eye = [
                target[0] + 10.0 * pitch.cos() * yaw.sin(),
                target[1] + 10.0 * pitch.sin(),
                target[2] + 10.0 * pitch.cos() * yaw.cos(),
            ];
            let view = look_at(eye, target, UP);
            let point = [target[0], target[1], target[2], 1.0];
            let viewed = transform(&view, point);
            assert!(
                viewed[0].abs() < 1e-5 && viewed[1].abs() < 1e-5,
                "target: {viewed:?}"
            );
            assert!(
                (viewed[2] + 10.0).abs() < 1e-5,
                "target must be ten units in front"
            );
            let clip = transform(&multiply4(&projection, &view), point);
            assert!(clip[3] > 0.0);
            assert!((clip[0] / clip[3]).abs() < 1e-5 && (clip[1] / clip[3]).abs() < 1e-5);
            assert!((0.0..1.0).contains(&(clip[2] / clip[3])));
        }
    }
}
