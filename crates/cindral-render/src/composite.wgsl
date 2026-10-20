@group(0) @binding(0) var offscreen_color: texture_2d<f32>;
@group(0) @binding(1) var offscreen_sampler: sampler;

struct QuadOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> QuadOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );
    let xy = positions[vertex_index];
    var out: QuadOutput;
    out.position = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: QuadOutput) -> @location(0) vec4<f32> {
    return textureSample(offscreen_color, offscreen_sampler, in.uv);
}
