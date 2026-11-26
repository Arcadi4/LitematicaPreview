// wgpu port of the renderer.ts scene pass: `VERTEX_SOURCE` + `FRAGMENT_SOURCE`.
//
// Pipeline mirror of the WebGL 2 shader:
//   * the vertex shader decodes block-space positions the same way
//     (`meshOrigin + position * meshExtent + offset`, then the MVP matrix);
//   * the fragment shader keeps the exact cutout, lighting and alpha rules.
// sRGB decoding happens in the sampler (Rgba8UnormSrgb textures), so tint and
// lighting run in linear space like the WebGL renderer, which relied on its
// sRGB attachment. The sRGB surface encodes the linear result on write.

struct Camera {
    // Per frame: `projection * view`.
    view_projection: mat4x4<f32>,
    // The `-centre` translation that parks the model bounds at the origin.
    // renderer.ts applied this to the scene pass only; the grid pass used a
    // zero offset, which lives in its own uniform buffer.
    offset: vec3<f32>,
};

struct Part {
    // Batch origin/extent, or 0/1 for absolute float positions.
    origin: vec3<f32>,
    extent: vec3<f32>,
    // 0 = opaque, 1 = cutout, 2 = blend.
    alpha_mode: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> part: Part;
@group(1) @binding(1) var block_texture: texture_2d<f32>;
@group(1) @binding(2) var block_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_uv: vec2<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) tint: vec4<f32>,
};

// 24-byte compact vertex: UNORM16x4 position, float2 uv, SNORM8x4 normal,
// UNORM8x4 color. The normalized position is expanded by the part uniforms.
@vertex
fn vertex_compact(
    @location(0) packed_position: vec4<f32>,
    @location(1) tex_uv: vec2<f32>,
    @location(2) normal: vec4<f32>,
    @location(3) tint: vec4<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    let world = part.origin + packed_position.xyz * part.extent + camera.offset;
    output.position = camera.view_projection * vec4<f32>(world, 1.0);
    output.tex_uv = tex_uv;
    output.normal = normal;
    output.tint = tint;
    return output;
}

// 28-byte wide vertex: float3 absolute position, float2 uv, SNORM8x4 normal,
// UNORM8x4 color. The part uniforms are 0/1, so the position passes through.
@vertex
fn vertex_float(
    @location(0) position: vec3<f32>,
    @location(1) tex_uv: vec2<f32>,
    @location(2) normal: vec4<f32>,
    @location(3) tint: vec4<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    let world = part.origin + position * part.extent + camera.offset;
    output.position = camera.view_projection * vec4<f32>(world, 1.0);
    output.tex_uv = tex_uv;
    output.normal = normal;
    output.tint = tint;
    return output;
}

@fragment
fn fragment_scene(
    input: VertexOutput,
    @builtin(front_facing) front_facing: bool,
) -> @location(0) vec4<f32> {
    let base = textureSample(block_texture, block_sampler, input.tex_uv) * input.tint;
    if (part.alpha_mode == 1.0 && base.a < 0.5) {
        discard;
    }
    let normal = normalize(input.normal.xyz) * select(-1.0, 1.0, front_facing);
    let light = 0.68 + 0.32 * max(dot(normal, normalize(vec3<f32>(1.0, 1.4, 0.8))), 0.0);
    let alpha = select(1.0, base.a, part.alpha_mode == 2.0);
    return vec4<f32>(base.rgb * light, alpha);
}
