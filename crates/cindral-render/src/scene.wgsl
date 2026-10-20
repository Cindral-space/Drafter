struct Camera {
    view_projection: mat4x4<f32>,
    style: vec4<f32>,
    background: vec4<f32>,
    reserved: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var t_materials: texture_2d_array<f32>;
@group(0) @binding(2) var s_materials: sampler;

struct FaceOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) inherited_color: vec4<f32>,
    @location(3) explicit_material: f32,
    @location(4) inherited_enabled: f32,
    @location(5) dim: f32,
    @location(6) uv: vec2<f32>,
    @location(7) tex_layer: f32,
};

@vertex
fn vs_face(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) material: f32,
    @location(4) m0: vec4<f32>,
    @location(5) m1: vec4<f32>,
    @location(6) m2: vec4<f32>,
    @location(7) m3: vec4<f32>,
    @location(8) n0: vec4<f32>,
    @location(9) n1: vec4<f32>,
    @location(10) n2: vec4<f32>,
    @location(11) n3: vec4<f32>,
    @location(12) inst_mat: vec4<f32>,
    @location(13) inst_params: vec4<f32>,
) -> FaceOutput {
    var out: FaceOutput;
    // Degenerate/zeroed slot detection uses zero normal rather than color
    if dot(normal, normal) == 0.0 {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.normal = vec3<f32>(0.0, 0.0, 1.0);
        out.color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.inherited_color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.explicit_material = 0.0;
        out.inherited_enabled = 0.0;
        out.dim = 0.0;
        out.uv = vec2<f32>(0.0, 0.0);
        out.tex_layer = 0.0;
        return out;
    }

    let model_matrix = mat4x4<f32>(m0, m1, m2, m3);
    let normal_matrix = mat4x4<f32>(n0, n1, n2, n3);

    let world_pos = model_matrix * vec4<f32>(position, 1.0);
    out.position = camera.view_projection * world_pos;

    // Normal transformation using inverse-transpose normal matrix (handles mirror/non-uniform scale)
    let transformed_normal = (normal_matrix * vec4<f32>(normal, 0.0)).xyz;
    out.normal = transformed_normal;

    // Decode FaceVertex.material packed float: texture_layer * 2 + explicit_material_bit
    let mat_int = u32(material + 0.5);
    let face_layer = mat_int / 2u;
    let explicit_bit = mat_int % 2u;
    let explicit_material = f32(explicit_bit);

    // Instance texture layer stored in inst_params.z (InstanceData.parameters[2])
    let inst_layer = u32(inst_params.z + 0.5);

    // Determine active texture layer per material hierarchy:
    // 1. Explicit face material uses face texture layer.
    // 2. Inherited container material uses instance texture layer if set, or face layer.
    // 3. Otherwise face texture layer.
    var active_layer: u32 = face_layer;
    if explicit_bit > 0u {
        active_layer = face_layer;
    } else if inst_params.x > 0.5 {
        if inst_layer > 0u {
            active_layer = inst_layer;
        } else {
            active_layer = face_layer;
        }
    } else {
        active_layer = face_layer;
    }

    // Planar local UVs derived from position and dominant normal
    let an = abs(normal);
    var uv: vec2<f32>;
    if an.x >= an.y && an.x >= an.z {
        uv = position.yz;
    } else if an.y >= an.x && an.y >= an.z {
        uv = position.xz;
    } else {
        uv = position.xy;
    }

    out.color = color;
    out.inherited_color = inst_mat;
    out.explicit_material = explicit_material;
    out.inherited_enabled = inst_params.x;
    out.dim = inst_params.y;
    out.uv = uv;
    out.tex_layer = f32(active_layer);
    return out;
}

fn compute_face_shading(in: FaceOutput) -> vec4<f32> {
    var rgb: vec3<f32>;
    var alpha: f32;

    if in.explicit_material > 0.5 {
        // Explicit face material wins over inherited container material
        rgb = in.color.rgb;
        alpha = in.color.a;
    } else if in.inherited_enabled > 0.5 {
        // Inherited container material is used
        rgb = in.inherited_color.rgb;
        // Opacity multiplies face and inherited opacity only when inherited is used
        alpha = in.color.a * in.inherited_color.a;
    } else {
        // Default face color and opacity
        rgb = in.color.rgb;
        alpha = in.color.a;
    }

    // Sample sRGB texture array with repeat sampler and clamp layer within array bounds
    let num_layers = textureNumLayers(t_materials);
    let raw_layer = u32(in.tex_layer + 0.5);
    let layer_idx = min(raw_layer, num_layers - 1u);
    let tex_sample = textureSample(t_materials, s_materials, in.uv, layer_idx);

    // Multiplies RGB/alpha into inherited/explicit material selection
    if camera.style.x == 1.0 {
        rgb = rgb * tex_sample.rgb;
        alpha = alpha * tex_sample.a;
    }
    if camera.style.x == 3.0 { return vec4<f32>(camera.background.rgb, 1.0); }
    if camera.style.x == 4.0 { alpha = 0.22; }

    let len2 = dot(in.normal, in.normal);
    let n = select(vec3<f32>(0.0, 0.0, 1.0), in.normal * inverseSqrt(len2), len2 > 1.0e-12);
    let key_light = normalize(vec3<f32>(0.45, -0.35, 0.82));
    let fill_light = normalize(vec3<f32>(-0.55, 0.60, 0.58));
    let shade = 0.45 + 0.40 * abs(dot(n, key_light)) + 0.15 * abs(dot(n, fill_light));

    let dimmed_rgb = rgb * in.dim * shade;
    return vec4<f32>(dimmed_rgb, alpha);
}

@fragment
fn fs_face_opaque(in: FaceOutput) -> @location(0) vec4<f32> {
    let color = compute_face_shading(in);
    if color.a < 0.999 {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}

@fragment
fn fs_face_transparent(in: FaceOutput) -> @location(0) vec4<f32> {
    let color = compute_face_shading(in);
    if color.a >= 0.999 {
        discard;
    }
    // Return premultiplied alpha for offscreen composite pass
    return vec4<f32>(color.rgb * color.a, color.a);
}

struct EdgeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

struct EdgeInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(4) m0: vec4<f32>,
    @location(5) m1: vec4<f32>,
    @location(6) m2: vec4<f32>,
    @location(7) m3: vec4<f32>,
    @location(8) n0: vec4<f32>,
    @location(9) n1: vec4<f32>,
    @location(10) n2: vec4<f32>,
    @location(11) n3: vec4<f32>,
    @location(12) inst_mat: vec4<f32>,
    @location(13) inst_params: vec4<f32>,
};

fn edge_vertex(input: EdgeInput) -> EdgeOutput {
    var out: EdgeOutput;
    if dot(input.color, input.color) == 0.0 || (input.color.x < 0.0 && camera.style.y < 0.5) || (input.color.y < 0.0 && camera.style.z < 0.5) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.color = vec3<f32>(0.0, 0.0, 0.0);
        return out;
    }

    let model_matrix = mat4x4<f32>(input.m0, input.m1, input.m2, input.m3);
    let world_pos = model_matrix * vec4<f32>(input.position, 1.0);
    out.position = camera.view_projection * world_pos;
    let dim = input.inst_params.y;
    let profile = input.color.z < 0.0 && camera.style.w > 0.5;
    let ink = select(vec3<f32>(0.08, 0.09, 0.10), vec3<f32>(0.94, 0.95, 0.98), dot(camera.background.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) < 0.5);
    let base = ink * select(0.75, 1.0, profile);
    out.color = base * dim;
    return out;
}

@vertex
fn vs_edge(input: EdgeInput) -> EdgeOutput { return edge_vertex(input); }

@vertex
fn vs_profile_left(input: EdgeInput) -> EdgeOutput {
    var out = edge_vertex(input);
    if input.color.z >= 0.0 { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.x += -camera.reserved.x * out.position.w;
    return out;
}

@vertex
fn vs_profile_right(input: EdgeInput) -> EdgeOutput {
    var out = edge_vertex(input);
    if input.color.z >= 0.0 { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.x += camera.reserved.x * out.position.w;
    return out;
}

@vertex
fn vs_profile_up(input: EdgeInput) -> EdgeOutput {
    var out = edge_vertex(input);
    if input.color.z >= 0.0 { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.y += camera.reserved.y * out.position.w;
    return out;
}

@vertex
fn vs_profile_down(input: EdgeInput) -> EdgeOutput {
    var out = edge_vertex(input);
    if input.color.z >= 0.0 { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.y += -camera.reserved.y * out.position.w;
    return out;
}

@fragment
fn fs_edge(in: EdgeOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
