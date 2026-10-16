//! Minimal column-major vector and matrix math for the camera.
//!
//! Only the operations the WebGL renderer needed are ported: the `gl-matrix`
//! `perspective`, `lookAt` and `multiply` calls plus the small `vec3`
//! helpers used by the orbit/pan fitting math. Matrices are column-major, so
//! they can be uploaded to WGSL `mat4x4<f32>` uniforms (also column-major)
//! without transposition.
//!
//! One deliberate difference from `gl-matrix`: WebGL clip space is `z in
//! [-1, 1]` while WebGPU/wgpu is `z in [0, 1]`. [`perspective`] produces the
//! wgpu convention (near maps to 0, far maps to 1); every other routine is
//! identical to the WebGL math.

/// A 3-component vector; matches the `vec3` usages in renderer.ts.
pub type Vec3 = [f32; 3];

/// A 4x4 matrix in column-major order.
pub type Mat4 = [f32; 16];

/// The world-up vector the WebGL renderer orbited around.
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

/// A right-handed perspective matrix mapping near to 0 and far to 1 in NDC,
/// matching `1 / tan(fov / 2)` for the vertical field of view. `aspect` is
/// width over height.
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

/// A right-handed look-at matrix, identical to `gl-matrix`'s `lookAt`:
/// the camera looks down its negative Z axis towards `centre`.
pub fn look_at(eye: Vec3, centre: Vec3, up: Vec3) -> Mat4 {
    let z = normalize3(sub3(eye, centre));
    let x = normalize3(cross3(up, z));
    let y = cross3(z, x);
    [
        x[0],
        x[1],
        x[2],
        0.0, //
        y[0],
        y[1],
        y[2],
        0.0, //
        z[0],
        z[1],
        z[2],
        0.0,           //
        -dot3(x, eye), //
        -dot3(y, eye), //
        -dot3(z, eye), //
        1.0,
    ]
}

/// Returns `a * b` in the same convention as `gl-matrix`'s `multiply(out, a, b)`.
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

    #[test]
    fn perspective_maps_near_and_far_to_zero_and_one() {
        let m = perspective(28.0f32.to_radians(), 1.5, 0.01, 0.24);
        let near = [0.0, 0.0, -0.01, 1.0];
        let far = [0.0, 0.0, -0.24, 1.0];
        let clip = |v: [f32; 4]| {
            [
                m[0] * v[0] + m[4] * v[1] + m[8] * v[2] + m[12] * v[3],
                m[1] * v[0] + m[5] * v[1] + m[9] * v[2] + m[13] * v[3],
                m[2] * v[0] + m[6] * v[1] + m[10] * v[2] + m[14] * v[3],
                m[3] * v[0] + m[7] * v[1] + m[11] * v[2] + m[15] * v[3],
            ]
        };
        let (nc, fc) = (clip(near), clip(far));
        assert!(nc[3] > 0.0 && fc[3] > 0.0);
        assert!((nc[2] / nc[3]).abs() < 1e-6, "near must map to z = 0");
        assert!(
            ((fc[2] / fc[3]) - 1.0).abs() < 1e-4,
            "far must map to z = 1"
        );
    }

    #[test]
    fn multiply_matches_identity_order() {
        let identity: Mat4 = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let translation: Mat4 = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            5.0, 6.0, 7.0, 1.0,
        ];
        let both = multiply4(&translation, &identity);
        assert_eq!(both, translation);
    }

    #[test]
    fn look_at_puts_the_target_on_the_negative_z_axis() {
        let view = look_at([0.0, 5.0, 10.0], [0.0, 0.0, 0.0], UP);
        let target = [
            view[0] * 0.0 + view[4] * 0.0 + view[8] * 0.0 + view[12] * 1.0,
            view[1] * 0.0 + view[5] * 0.0 + view[9] * 0.0 + view[13] * 1.0,
            view[2] * 0.0 + view[6] * 0.0 + view[10] * 0.0 + view[14] * 1.0,
            view[3] * 0.0 + view[7] * 0.0 + view[11] * 0.0 + view[15] * 1.0,
        ];
        assert!(target[0].abs() < 1e-6 && target[1].abs() < 1e-6);
        assert!(target[2] < 0.0, "the target sits in front of the camera");
    }
}
