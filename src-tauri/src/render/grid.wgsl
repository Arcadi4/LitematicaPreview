// wgpu port of the renderer.ts grid pass: the `isGrid` branch of
// `FRAGMENT_SOURCE`, drawn with a zero camera offset.
//
// The grid geometry is built once from the model's metadata bounds and kept in
// a dedicated vertex buffer; the WebGL renderer reused its scene program with
// `isGrid` set, which is reproduced here as a separate pipeline so the grid can
// carry a position-only vertex layout.

struct Camera {
    view_projection: mat4x4<f32>,
    offset: vec3<f32>,
};

// Shares the scene pass's camera uniform buffer and layout, but the grid binds
// the buffer instance whose offset stays zero.
@group(0) @binding(0) var<uniform> camera: Camera;

@vertex
fn vertex_grid(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.view_projection * vec4<f32>(position, 1.0);
}

@fragment
fn fragment_grid() -> @location(0) vec4<f32> {
    // The same constant the WebGL fragment shader emitted for grid lines. The
    // sRGB surface target encodes this linear value on write.
    return vec4<f32>(0.020, 0.030, 0.042, 1.0);
}
