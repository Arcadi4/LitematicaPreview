struct Layout {
    window: vec4f, // width, height, night, animation time
    pane: vec4f,
    dock: vec4f,
    selection: vec4f,
    nav: vec4f,
    back: vec4f,
    aim: vec4f,
}
@group(0) @binding(0) var<uniform> ui: Layout;
@group(0) @binding(1) var bodies: texture_2d<f32>;
@group(0) @binding(2) var heights: texture_2d<u32>;
@group(0) @binding(3) var horizon_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var horizon: texture_2d<f32>;
@group(0) @binding(5) var horizon_sampler: sampler;

const SKYLINE: f32 = 0.625;
const MAX_PAN: vec2f = vec2f(20.0, 14.0);

struct Vertex {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) ray: vec3f,
    @location(3) @interpolate(flat) source: vec2f,
}
@vertex fn vs(@builtin(vertex_index) i: u32, @builtin(instance_index) layer: u32) -> Vertex {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    let size = ui.pane.zw;
    let focal = max(size.x, size.y);
    let look = ui.aim.xy * min(MAX_PAN, size*0.038) / focal;
    let forward = normalize(vec3f(look, 1.0));
    let right = normalize(vec3f(1.0, 0.0, -look.x));
    let down = cross(forward, right);
    let film = (uv - 0.5) * size / focal;
    let ray = forward + film.x*right + film.y*down;
    let source = ui.pane.xy + 0.5*size + focal*vec2f(right.z, down.z)/forward.z;
    return Vertex(vec4f(uv * vec2f(2,-2) + vec2f(-1,1), 0, 1), uv, layer, ray, source);
}

fn linear(c: vec3f) -> vec3f { return c * (c * (0.3053 * c + 0.6822) + 0.0125); }
fn palette(day: vec3f, night: vec3f) -> vec3f { return linear(mix(day, night, ui.window.z)); }
fn hash(p: vec2f) -> f32 {
    var v = fract(vec3f(p.xyx) * vec3f(0.1031, 0.1030, 0.0973));
    v += dot(v, v.yzx + 33.33);
    return fract((v.x + v.y) * v.z);
}
fn light_color() -> vec3f { return palette(vec3f(1.0, 0.81, 0.44), vec3f(0.60, 0.78, 1.0)); }

fn stars(p: vec2f) -> f32 {
    let tile = floor(p / 30.0);
    let seed = hash(tile);
    let center = (tile + 0.1 + 0.8 * vec2f(hash(tile + 7.2), hash(tile - 3.1))) * 30.0;
    let d = abs(p - center);
    let core = 1.0 - smoothstep(0.3, 1.25, max(d.x, d.y));
    let twinkle = 0.84 + 0.16*cos(ui.window.w*0.72 + 26.0*seed);
    return core * smoothstep(0.62, 1.0, seed) * twinkle;
}

fn shafts(angle: f32) -> f32 {
    let a = angle + 0.023*sin(ui.window.w*0.21 + 2.0*angle)
                  + 0.014*sin(ui.window.w*0.37 - 3.0*angle);
    let fan = 0.5 + 0.5 * sin(17.0*a + 2.8*sin(5.0*a) + 1.1*cos(9.0*a));
    let streak = 0.5 + 0.5 * cos(31.0*a + 1.7*sin(7.0*a));
    return 0.54*pow(fan, 2.8) + 0.28*pow(streak, 7.0);
}

fn sky(uv: vec2f, p: vec2f, body_size: f32) -> vec3f {
    let night = ui.window.z;
    let horizon = smoothstep(0.0, 0.74, uv.y);
    let zenith = palette(vec3f(0.32, 0.56, 0.68), vec3f(0.028, 0.038, 0.098));
    let mist = palette(vec3f(0.94, 0.78, 0.60), vec3f(0.23, 0.30, 0.46));
    var color = mix(zenith, mist, horizon * horizon);
    let r = length(p) / body_size;
    let angle = atan2(p.y, p.x + 0.0001);
    let beams = shafts(angle) * exp(-0.63*r);
    let scatter = 0.52*exp(-0.52*r) + 1.0/(1.0 + 3.6*r*r);
    let warm = palette(vec3f(1.0, 0.69, 0.26), vec3f(0.53, 0.65, 0.91));
    color += warm * (scatter + beams * mix(1.6, 0.32, night));
    color = mix(color, vec3f(1.0, 0.99, 0.93), (1.0-night) * 0.62 * exp(-1.7*r));
    if night > 0.5 {
        color += vec3f(0.63,0.77,1.0) * stars(p) * smoothstep(1.0,3.0,r) * (1.0 - 0.85*horizon);
    }
    let texel = p / body_size + 0.5;
    if all(texel >= vec2f(0.0)) && all(texel < vec2f(1.0)) {
        let coord = vec2i(texel * 8.0) + vec2i(i32(night) * 8, 0);
        color = textureLoad(bodies, coord, 0).rgb + night*vec3f(0.060,0.072,0.11);
    }
    return color;
}

// Separate ground and crown slabs leave open space under the overhanging branches.
fn landscape(uv: vec2f) -> vec4f {
    let down = uv.y - SKYLINE;
    let x = (uv.x - 0.5) * ui.pane.z / ui.pane.w;
    let ray = vec3f(x * 0.97 + 0.26, -down, 1.06 - x * 0.24);
    let origin = vec3f(33.35, 23.5, 16.37);
    var t = (origin.y - 22.0) / down;
    let start = origin.xz + t * ray.xz;
    var cell = vec2i(floor(start));
    let step_dir = vec2i(sign(ray.xz));
    let stride = 1.0 / max(abs(ray.xz), vec2f(0.00001));
    var next = t + select(fract(start), 1.0-fract(start), ray.xz > vec2f(0.0)) * stride;
    var face = vec3f(0, 0, -1);
    for (var i = 0; i < 194 && t < 138.0; i++) {
        let column = textureLoad(heights, cell & (vec2i(textureDimensions(heights)) - 1), 0).rg;
        let ground = f32(column.x);
        let crown = f32(column.y & 127u);
        let trunk = column.y >= 128u;
        let y = origin.y - t * down;
        let leaf = crown > ground && (y >= crown - 2.0 || trunk);
        let top = select(ground, crown, leaf);
        let hit = max(t, (origin.y - top) / down);
        if hit <= min(next.x, next.y) {
            let pos = origin + hit * ray;
            let normal = select(face, vec3f(0,1,0), y > top);
            let grass = normal.y > 0.5 || pos.y > ground - 0.32;
            var albedo = select(vec3f(0.40,0.29,0.22), vec3f(0.39,0.48,0.28), grass);
            if leaf { albedo = select(vec3f(0.17,0.31,0.28), vec3f(0.25,0.20,0.15), trunk && pos.y < crown - 2.0); }
            let grain = hash(floor(pos.xz*4.0) + floor(pos.y*4.0)*0.37);
            let sunward = max(0.0, dot(normal, normalize(vec3f(0.24,0.44,1.0))));
            let shadow_column = textureLoad(heights, (cell + vec2i(1,2)) & (vec2i(textureDimensions(heights))-1), 0).rg;
            let blocker = f32(max(shadow_column.x, shadow_column.y & 127u));
            let visibility = smoothstep(-0.6, 1.4, pos.y + 1.0 - blocker);
            let irradiance = palette(vec3f(0.68,0.71,0.74), vec3f(0.30,0.39,0.59)) + light_color()*sunward*visibility;
            let shaded = linear(albedo) * irradiance * (0.88 + grain*0.22);
            let opacity = exp(-hit * hit * 0.00013) * (1.0-smoothstep(116.0,138.0,hit));
            return vec4f(shaded * opacity, opacity);
        }
        if next.x < next.y {
            t = next.x; next.x += stride.x; cell.x += step_dir.x;
            face = vec3f(-f32(step_dir.x),0,0);
        } else {
            t = next.y; next.y += stride.y; cell.y += step_dir.y;
            face = vec3f(0,0,-f32(step_dir.y));
        }
    }
    return vec4f(0);
}

// Overscan keeps camera turns inside the cached terrain, including at the corners.
fn horizon_rect() -> vec4f {
    let guard = 2.0*max(MAX_PAN.x, MAX_PAN.y);
    let top = SKYLINE*ui.pane.w;
    return vec4f(-guard, top, ui.pane.z + 2.0*guard, ui.pane.w - top + guard);
}

@compute @workgroup_size(8, 8)
fn bake_horizon(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(horizon_out);
    if any(id.xy >= size) { return; }
    let uv = (vec2f(id.xy) + 0.5) / vec2f(size);
    let rect = horizon_rect();
    textureStore(horizon_out, id.xy, landscape((rect.xy + uv*rect.zw)/ui.pane.zw));
}

fn rounded(p: vec2f, r: vec4f, radius: f32) -> f32 {
    let q = abs(p - r.xy - r.zw*0.5) - r.zw*0.5 + radius;
    return length(max(q, vec2f(0.0))) + min(max(q.x,q.y),0.0) - radius;
}
fn pane_edge(p: vec2f) -> f32 {
    return rounded(p,ui.pane,8.0);
}
fn pane_coverage(edge: f32) -> f32 {
    return 1.0-smoothstep(-0.75,0.25,edge);
}
fn over(fg: vec4f, bg: vec4f) -> vec4f { return fg + bg * (1.0 - fg.a); }
fn ink(c: vec3f, alpha: f32) -> vec4f { return vec4f(c * alpha, alpha); }

// The projected shadow and the bevel normal use the same raised surface and area-light position.
fn relief(p: vec2f, r: vec4f, radius: f32, height: f32, source: vec2f) -> vec4f {
    if r.z <= 0.0 { return vec4f(0); }
    let v = p - r.xy - 0.5*r.zw;
    if any(abs(v) > r.zw*0.5 + 38.0) { return vec4f(0); }
    let d = rounded(p,r,radius);
    let light = source - p;
    let lamp_z = max(80.0, ui.pane.w * 0.25);
    let eye = ui.aim.xy*0.85;
    let offset = height * (light / max(lamp_z, length(light)*0.62) + eye);
    let penumbra = 1.8 + length(offset)*0.74;
    let penumbra_mask = 1.0 - smoothstep(-penumbra, penumbra, rounded(p + offset, r, radius));
    let contact = 1.0 - smoothstep(-0.8, 2.0, d);
    let coverage = 1.0 - smoothstep(-0.55,0.55,d);
    let q = abs(v) - r.zw*0.5 + radius;
    let outward = sign(v) * select(select(vec2f(0,1),vec2f(1,0),q.x>q.y),
        normalize(max(q,vec2f(0.0001))), max(q.x,q.y)>0.0);
    let facing = dot(outward, normalize(light + vec2f(0.001)));
    let edge = exp(-abs(d + 0.85) * 1.1);
    let bevel = exp(-max(-d,0.0) / (height + 1.2));
    let falloff = 1.0 / (1.0 + dot(light,light)/(lamp_z*lamp_z*8.0));
    let incident = falloff * (0.70 + 0.65*shafts(atan2(-light.y,-light.x)));
    let diffuse = (0.09 + 0.49*max(facing,0.0)*bevel) * incident;
    let normal = normalize(vec3f(0.32*v/(0.5*r.zw) + 0.68*bevel*outward, 1.0));
    let halfway = normalize(normalize(vec3f(light,lamp_z)) + normalize(vec3f(eye,1.0)));
    let reflection = pow(max(dot(normal,halfway),0.0),24.0) * (0.16 + 0.60*bevel);
    let specular = ((0.12 + 1.4*pow(max(facing,0.0),2.0))*edge + reflection) * incident;
    let bounce = palette(vec3f(1.0,0.69,0.30),vec3f(0.49,0.70,1.0));
    let highlight = over(ink(mix(light_color(),vec3f(1.0),0.45), min(0.95,specular)),
                         ink(bounce, diffuse)) * coverage;
    let unlit = coverage * 0.19*max(-facing,0.0)*bevel;
    let shadow = (1.0-coverage) * (0.36*penumbra_mask + 0.20*contact);
    return over(highlight, ink(vec3f(0.005,0.009,0.014), shadow + unlit));
}

@fragment fn fs(v: Vertex) -> @location(0) vec4f {
    let size = ui.pane.zw;
    let source = v.source;
    if v.layer == 1u {
        let p = v.uv * ui.window.xy;
        let edge = pane_edge(p);
        let inside = pane_coverage(edge);
        if inside == 1.0 {
            return over(relief(p,ui.selection,6.0,2.0,source), relief(p,ui.dock,10.0,5.0,source));
        }
        let reflection = source + ui.aim.xy * (0.16*min(size.x,size.y));
        let delta = (p-reflection) / max(180.0,min(size.x,size.y)*0.56);
        let distance = max(0.0,edge);
        let rim = 0.58*exp(-distance*0.52) + 0.19*exp(-distance/27.0);
        let spill = ink(light_color(), (1.0-inside)*(0.28 + rim)*exp(-0.91*dot(delta,delta)));
        return over(relief(p,ui.back,6.0,2.8,source),over(relief(p,ui.nav,6.0,3.2,source),spill));
    }
    let p = v.uv * size;
    let centered = v.ray.xy/v.ray.z * max(size.x,size.y);
    let world_px = centered + 0.5*size;
    let body_size = clamp(min(size.x,size.y)*0.110, 36.0, 80.0);
    var color = sky(world_px/size,centered,body_size);
    let rect = horizon_rect();
    if world_px.y >= rect.y {
        let land = textureSampleLevel(horizon,horizon_sampler,(world_px-rect.xy)/rect.zw,0.0);
        color = land.rgb + (1.0-land.a) * color;
    }
    color *= 1.0 - 0.60*smoothstep(0.76, 1.0, v.uv.y);
    let dock = rounded(p + ui.pane.xy, ui.dock, 10.0);
    if dock < 1.0 {
        let metal = palette(vec3f(0.45,0.37,0.24),vec3f(0.17,0.21,0.32));
        color = mix(color,metal,1.0-smoothstep(-0.5,0.5,dock));
    }
    return ink(color,pane_coverage(pane_edge(p + ui.pane.xy)));
}
