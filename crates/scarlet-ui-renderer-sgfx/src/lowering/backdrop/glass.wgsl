const CONTOUR_SHADOW_GAIN: f32 = 1.0;
struct Parameters {
    rect: vec4<f32>,
    extent: vec4<f32>,
    tint: vec4<f32>,
    optics: vec4<f32>,
    capture: vec4<f32>,
    kernel: array<vec4<f32>, 65>,
};
@group(0) @binding(0) var<uniform> p: Parameters;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;

fn clip_position(position: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(position.x * 2.0 / p.extent.x - 1.0,
        1.0 - position.y * 2.0 / p.extent.y, 0.0, 1.0);
}
@vertex fn blur_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
        vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    return clip_position(p.capture.xy + corners[index] * p.capture.zw);
}
@vertex fn glass_vertex(@location(0) position: vec4<f32>) -> @builtin(position) vec4<f32> {
    return clip_position(position.xy);
}
fn blur(position: vec2<f32>, axis: vec2<f32>) -> vec4<f32> {
    var result = vec4<f32>(0.0);
    for (var i = 0u; i < u32(p.optics.w); i += 1u) {
        let sample_position = position + axis * p.kernel[i].x;
        result += textureSampleLevel(source, linear_sampler, sample_position / p.extent.xy, 0.0) * p.kernel[i].y;
    }
    return result;
}
@fragment fn horizontal(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return blur(position.xy, vec2(1.0, 0.0));
}
@fragment fn vertical(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return blur(position.xy, vec2(0.0, 1.0));
}
// Signed inward distance and inward normal, matching core::glass_edge.
fn edge(position: vec2<f32>) -> vec3<f32> {
    let half_size = p.rect.zw * 0.5;
    let radius = clamp(p.extent.w, 0.0, min(half_size.x, half_size.y));
    let local = position - p.rect.xy - half_size;
    let q = abs(local) - half_size + radius;
    let signs = select(vec2(-1.0), vec2(1.0), local >= vec2(0.0));
    if (q.x > 0.0 && q.y > 0.0) {
        return vec3(radius - length(q), -signs * normalize(q));
    }
    if (q.x > q.y) { return vec3(radius - q.x, -signs.x, 0.0); }
    return vec3(radius - q.y, 0.0, -signs.y);
}
fn coverage(position: vec2<f32>) -> f32 {
    return clamp(edge(position).x + 0.5, 0.0, 1.0) * p.optics.z;
}
@fragment fn erase(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4(0.0, 0.0, 0.0, coverage(position.xy));
}
fn specular_lobe(cosine: f32) -> f32 {
    let x = clamp(cosine, 0.0, 1.0);
    let x2 = x * x;
    let x4 = x2 * x2;
    let x8 = x4 * x4;
    return x8;
}
@fragment fn glass(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let e = edge(position.xy);
    let scale = p.extent.z;
    let bezel = max(0.0, min(clamp(p.extent.w * 1.5, 16.0 * scale, 24.0 * scale), min(p.rect.z, p.rect.w) * 0.35));
    var sample_position = position.xy;
    if (bezel > 0.0 && e.x >= 0.0 && e.x < bezel) {
        let t = clamp(1.0 - e.x / bezel, 0.0, 0.98);
        let sin_i = t * t;
        let cos_i = sqrt(1.0 - sin_i * sin_i);
        let sin_t = sin_i / 1.5;
        let cos_t = sqrt(1.0 - sin_t * sin_t);
        let tangent = (sin_i * cos_t - cos_i * sin_t) / (cos_i * cos_t + sin_i * sin_t);
        let thickness = min(bezel, 12.0 * scale);
        sample_position += e.yz * min(tangent * thickness * 1.65, thickness * 1.4) * p.optics.y;
    }
    let background = textureSampleLevel(source, linear_sampler, sample_position / p.extent.xy, 0.0);
    var material = vec4(p.tint.rgb * p.tint.a, p.tint.a) + background * (1.0 - p.tint.a);
    let reflection_t = clamp(1.0 - max(e.x, 0.0) / max(bezel, 0.0001), 0.0, 1.0);
    let normal_xy = reflection_t * reflection_t;
    let normal_z = sqrt(max(1.0 - normal_xy * normal_xy, 0.0));
    let direction = clamp(dot(e.yz, vec2(0.6, 0.8)), -1.0, 1.0);
    let key = specular_lobe(direction * normal_xy * 0.85 + normal_z * 0.5267827);
    let fill = specular_lobe(-direction * normal_xy * 0.94 + normal_z * 0.34117445);
    let grazing = 1.0 - normal_z;
    let grazing2 = grazing * grazing;
    let fresnel = grazing2 * grazing2 * grazing;
    // Match core::glass_surface_reflection: dark sides and finer bright horizontal rims.
    let surface = (0.012 * fresnel + (0.16 * key + 0.06 * fill) * normal_xy) * e.z * e.z;
    let side = e.y * e.y;
    let width = max(0.0001, min((1.25 + 0.50 * side) * scale, bezel * 0.10));
    let opacity = 0.05 + 0.13 * side + 0.025 * max(-e.z, 0.0);
    let fade = clamp(1.0 - max(e.x, 0.0) / width, 0.0, 1.0);
    let line_light = (0.34 - 0.06 * max(-e.z, 0.0)) * e.z * e.z;
    let shoulder = surface + line_light * fade * fade * (3.0 - 2.0 * fade);
    let top_pixels = max(floor(scale + 0.5), 1.0);
    let side_pixels = max(floor(2.0 * scale + 0.5), top_pixels + 1.0);
    let line_width = min(top_pixels + (side_pixels - top_pixels) * side, bezel);
    let line = clamp(line_width - max(e.x, 0.0) + 0.5, 0.0, 1.0);
    let reflection = (shoulder * (1.0 - line) - opacity * CONTOUR_SHADOW_GAIN * line) * p.optics.y;
    let light = vec4(select(vec3(0.0), vec3(1.0), reflection >= 0.0), 1.0);
    material += (light - material) * abs(reflection);
    return material * coverage(position.xy);
}
