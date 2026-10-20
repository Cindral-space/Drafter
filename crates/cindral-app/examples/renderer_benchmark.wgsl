// Shaded static heightfield for the large-mesh renderer benchmark.
struct Uniforms {
    view_projection: mat4x4<f32>,
    light_direction: vec4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) height: f32,
};

@vertex fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.position = uniforms.view_projection * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.height = position.z;
    return out;
}

@fragment fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let diffuse = max(dot(n, normalize(uniforms.light_direction.xyz)), 0.0);
    let t = clamp(in.height * 0.025 + 0.5, 0.0, 1.0);
    let albedo = mix(vec3<f32>(0.25, 0.45, 0.30), vec3<f32>(0.85, 0.80, 0.65), t);
    return vec4<f32>(albedo * (0.2 + 0.8 * diffuse), 1.0);
}
