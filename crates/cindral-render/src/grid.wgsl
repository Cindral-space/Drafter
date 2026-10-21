struct Camera {
    view_projection: mat4x4<f32>,
    viewport: vec4<f32>,
};
@group(0) @binding(0) var<uniform> camera: Camera;
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) @interpolate(flat) dot_direction: vec2<f32>,
};
@vertex fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) dotted_axis: vec3<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.position = camera.view_projection * vec4<f32>(position, 1.0);
    out.color = color;
    // Differentiate the perspective divide along the axis. This remains valid
    // when the origin is outside the view or lies on the camera plane.
    let axis = camera.view_projection * vec4<f32>(dotted_axis, 0.0);
    let projected = (axis.xy * out.position.w - out.position.xy * axis.w)
        * camera.viewport.xy * vec2<f32>(1.0, -1.0);
    out.dot_direction = projected / max(length(projected), 0.000001);
    return out;
}
@fragment fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Two-pixel dots and four-pixel gaps, measured in logical screen pixels.
    if length(in.dot_direction) > 0.5 {
        let distance = dot(in.position.xy / camera.viewport.z, in.dot_direction);
        if fract(distance / 6.0) >= 1.0 / 3.0 {
            discard;
        }
    }
    return vec4<f32>(in.color, 1.0);
}
