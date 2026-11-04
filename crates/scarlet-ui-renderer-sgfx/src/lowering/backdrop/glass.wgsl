struct Parameters {
    rect: vec4<f32>,
    extent: vec4<f32>,
    tint: vec4<f32>,
    optics: vec4<f32>,
    capture: vec4<f32>,
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
    let weights = array<f32, 9>(1.0, 8.0, 28.0, 56.0, 70.0, 56.0, 28.0, 8.0, 1.0);
    var result = vec4<f32>(0.0);
    for (var i = 0u; i < 9u; i += 1u) {
        let sample_position = position + axis * (f32(i) - 4.0) * p.optics.x * 0.25;
        result += textureSampleLevel(source, linear_sampler, sample_position / p.extent.xy, 0.0) * weights[i];
    }
    return result / 256.0;
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
@fragment fn glass(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let e = edge(position.xy);
    let scale = p.extent.z;
    let bezel = max(0.0, min(min(p.extent.w, 12.0 * scale), min(p.rect.z, p.rect.w) * 0.25));
    var sample_position = position.xy;
    if (bezel > 0.0 && e.x >= 0.0 && e.x < bezel) {
        let t = clamp(1.0 - e.x / bezel, 0.0, 0.98);
        let sin_i = t * t;
        let cos_i = sqrt(1.0 - sin_i * sin_i);
        let sin_t = sin_i / 1.5;
        let cos_t = sqrt(1.0 - sin_t * sin_t);
        let tangent = (sin_i * cos_t - cos_i * sin_t) / (cos_i * cos_t + sin_i * sin_t);
        sample_position += e.yz * min(tangent * bezel * 1.65, bezel * 1.4) * p.optics.y;
    }
    let background = textureSampleLevel(source, linear_sampler, sample_position / p.extent.xy, 0.0);
    var material = vec4(p.tint.rgb * p.tint.a, p.tint.a) + background * (1.0 - p.tint.a);
    let band = max(0.0, 1.0 - abs((e.x / scale - 0.55) / 0.7));
    let light = clamp(dot(e.yz, vec2(0.6, 0.8)), -1.0, 1.0);
    let reflection = band * select(0.065 * light, 0.30 * light, light >= 0.0) * p.optics.y;
    let reflected = select(vec4(0.0, 0.0, 0.0, 1.0), vec4(1.0), reflection >= 0.0);
    material += (reflected - material) * abs(reflection);
    return material * coverage(position.xy);
}
