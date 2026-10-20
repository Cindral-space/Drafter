struct Camera {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct FaceOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_face(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
) -> FaceOutput {
    var out: FaceOutput;
    if dot(color, color) == 0.0 {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.normal = vec3<f32>(0.0, 0.0, 1.0);
        out.color = vec3<f32>(0.0, 0.0, 0.0);
        return out;
    }
    out.position = camera.view_projection * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.color = color;
    return out;
}

@fragment
fn fs_face(in: FaceOutput) -> @location(0) vec4<f32> {
    let len2 = dot(in.normal, in.normal);
    let n = select(vec3<f32>(0.0, 0.0, 1.0), in.normal * inverseSqrt(len2), len2 > 1.0e-12);
    let key_light = normalize(vec3<f32>(0.45, -0.35, 0.82));
    let fill_light = normalize(vec3<f32>(-0.55, 0.60, 0.58));
    let shade = 0.45 + 0.40 * abs(dot(n, key_light)) + 0.15 * abs(dot(n, fill_light));
    return vec4<f32>(in.color * shade, 1.0);
}

struct EdgeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_edge(
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
) -> EdgeOutput {
    var out: EdgeOutput;
    if dot(color, color) == 0.0 {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.color = vec3<f32>(0.0, 0.0, 0.0);
        return out;
    }
    out.position = camera.view_projection * vec4<f32>(position, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_edge(in: EdgeOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
