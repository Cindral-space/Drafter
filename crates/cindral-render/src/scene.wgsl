struct Camera {
    view_projection: mat4x4<f32>,
    style: vec4<f32>,
    background: vec4<f32>,
    reserved: vec4<f32>,
    view_origin: vec4<f32>,
    section: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var t_materials: texture_2d_array<f32>;
@group(0) @binding(2) var s_materials: sampler;
@group(0) @binding(6) var t_params: texture_2d<f32>;

fn load_material_mapping(index: u32) -> array<vec4<f32>, 3> {
    return array<vec4<f32>, 3>(
        textureLoad(t_params, vec2<i32>(0, i32(index)), 0),
        textureLoad(t_params, vec2<i32>(1, i32(index)), 0),
        textureLoad(t_params, vec2<i32>(2, i32(index)), 0),
    );
}

struct Sun { matrix: mat4x4<f32>, direction: vec4<f32>, parameters: vec4<f32>, section: vec4<f32>, ground: vec4<f32> };
@group(0) @binding(3) var<uniform> sun: Sun;
@group(0) @binding(4) var shadow_map: texture_depth_2d;
@group(0) @binding(5) var shadow_sampler: sampler_comparison;
fn sunlight(world:vec3<f32>) -> f32 {
    let p=sun.matrix*vec4<f32>(world,1.0);
    let uv=vec2<f32>(p.x*0.5+0.5,0.5-p.y*0.5);
    let texel=1.0/vec2<f32>(textureDimensions(shadow_map));
    var lit=0.0;
    var count=0.0;
    let low=textureDimensions(shadow_map).x<=1024u;
    for(var y=-1;y<=1;y+=1) {for(var x=-1;x<=1;x+=1) {
        if low && (x==1 || y==1) {continue;}
        let offset=vec2<f32>(f32(x),f32(y))+select(vec2<f32>(0.0),vec2<f32>(0.5),low);
        lit+=textureSampleCompareLevel(shadow_map,shadow_sampler,uv+offset*texel,p.z-0.0003);
        count+=1.0;
    }}
    if any(uv<vec2<f32>(0.0)) || any(uv>vec2<f32>(1.0)) || p.z<0.0 || p.z>1.0 {return 1.0;}
    return lit/count;
}

struct FaceOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) dim: f32,
    @location(3) uv: vec2<f32>,
    @location(4) tex_layer: f32,
    @location(5) section_distance: f32,
    @location(6) world: vec3<f32>,
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
    out.section_distance = 0.0;
    out.world=vec3<f32>(0.0);
    // Degenerate/zeroed slot detection uses zero normal rather than color
    if dot(normal, normal) == 0.0 {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.normal = vec3<f32>(0.0, 0.0, 1.0);
        out.color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        out.dim = 0.0;
        out.uv = vec2<f32>(0.0, 0.0);
        out.tex_layer = 0.0;
        return out;
    }

    let model_matrix = mat4x4<f32>(m0, m1, m2, m3);
    let normal_matrix = mat4x4<f32>(n0, n1, n2, n3);

    let world_pos = model_matrix * vec4<f32>(position, 1.0);
    out.position = camera.view_projection * world_pos;
    out.world=world_pos.xyz;
    out.section_distance = select(dot(camera.section, world_pos), 0.0, material < 0.0);

    // Normal transformation using inverse-transpose normal matrix (handles mirror/non-uniform scale)
    let transformed_normal = (normal_matrix * vec4<f32>(normal, 0.0)).xyz;
    out.normal = transformed_normal;

    // Decode FaceVertex.material packed float: material_index * 2 + explicit_bit.
    // Index 0 keeps the vertex-baked legacy color path.
    let mat_int = u32(max(material, 0.0) + 0.5);
    let index = mat_int / 2u;
    let explicit_bit = mat_int % 2u;

    var resolved_color = color;
    var active_layer: u32 = 0u;
    var uv_scale = vec2<f32>(1.0);
    var uv_rotation = 0.0;
    var uv_flip = vec2<f32>(1.0, 1.0);

    if explicit_bit > 0u && index > 0u {
        let record = load_material_mapping(index);
        resolved_color = record[0];
        active_layer = u32(record[1].x + 0.5);
        uv_scale = record[1].yz;
        uv_rotation = record[1].w;
        uv_flip = record[2].xy;
    } else if explicit_bit == 0u && inst_params.x > 0.5 {
        // Inherited container material from the table; face opacity still applies.
        let inst_index = u32(inst_params.z + 0.5);
        if inst_index > 0u {
            let record = load_material_mapping(inst_index);
            resolved_color = vec4<f32>(record[0].rgb, record[0].a * color.a);
            active_layer = u32(record[1].x + 0.5);
            uv_scale = record[1].yz;
            uv_rotation = record[1].w;
            uv_flip = record[2].xy;
        }
    }

    // Planar local UVs derived from position and dominant normal, then the
    // material mapping: flip, rotate about the mapping origin, scale to
    // real-world texture size.
    let an = abs(normal);
    var uv: vec2<f32>;
    if an.x >= an.y && an.x >= an.z {
        uv = position.yz;
    } else if an.y >= an.x && an.y >= an.z {
        uv = position.xz;
    } else {
        uv = position.xy;
    }
    uv = uv * uv_flip;
    let c = cos(uv_rotation);
    let s = sin(uv_rotation);
    uv = vec2<f32>(uv.x * c - uv.y * s, uv.x * s + uv.y * c) * uv_scale;

    out.color = resolved_color;
    out.dim = inst_params.y;
    out.uv = uv;
    out.tex_layer = f32(active_layer);
    return out;
}

fn compute_face_shading(in: FaceOutput) -> vec4<f32> {
    var rgb: vec3<f32> = in.color.rgb;
    var alpha: f32 = in.color.a;

    // Sample sRGB texture array with repeat sampler and clamp layer within array bounds
    let num_layers = textureNumLayers(t_materials);
    let raw_layer = u32(in.tex_layer + 0.5);
    let layer_idx = min(raw_layer, num_layers - 1u);
    let tex_sample = textureSample(t_materials, s_materials, in.uv, layer_idx);

    // Multiplies RGB/alpha into the resolved material color
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

    var lighting=shade;
    if sun.parameters.w>0.5 {
        var visibility=1.0;
        if sun.parameters.z>0.5 {visibility=sunlight(in.world);}
        if sun.direction.z<=0.0 {visibility=0.0;}
        lighting=mix(sun.parameters.y,sun.direction.w,visibility)*(0.55+0.45*abs(dot(n,sun.direction.xyz)));
    }
    let dimmed_rgb = rgb * in.dim * lighting;
    return vec4<f32>(dimmed_rgb, alpha);
}

@fragment
fn fs_face_opaque(in: FaceOutput) -> @location(0) vec4<f32> {
    let color = compute_face_shading(in);
    if in.section_distance < -0.00001 { discard; }
    if color.a < 0.999 {
        discard;
    }
    return vec4<f32>(color.rgb, 1.0);
}

@fragment
fn fs_face_transparent(in: FaceOutput) -> @location(0) vec4<f32> {
    let color = compute_face_shading(in);
    if in.section_distance < -0.00001 { discard; }
    if color.a >= 0.999 {
        discard;
    }
    // Return premultiplied alpha for offscreen composite pass
    return vec4<f32>(color.rgb * color.a, color.a);
}

struct EdgeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) section_distance: f32,
};

struct EdgeInput {
    @location(2) normal_a: vec3<f32>,
    @location(3) normal_b: vec3<f32>,
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

fn is_profile(input: EdgeInput) -> bool {
    if input.color.z < 0.0 { return true; }
    let model = mat4x4<f32>(input.m0, input.m1, input.m2, input.m3);
    let normals = mat4x4<f32>(input.n0, input.n1, input.n2, input.n3);
    let world = model * vec4<f32>(input.position, 1.0);
    let direction = camera.view_origin.xyz - world.xyz * camera.view_origin.w;
    let a = (normals * vec4<f32>(input.normal_a, 0.0)).xyz;
    let b = (normals * vec4<f32>(input.normal_b, 0.0)).xyz;
    return dot(a, direction) * dot(b, direction) < 0.0;
}

fn edge_vertex(input: EdgeInput) -> EdgeOutput {
    var out: EdgeOutput;
    out.section_distance = 0.0;
    if dot(input.color, input.color) == 0.0 || (input.color.x < 0.0 && camera.style.y < 0.5) || (input.color.y < 0.0 && camera.style.z < 0.5) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.color = vec3<f32>(0.0, 0.0, 0.0);
        return out;
    }

    let model_matrix = mat4x4<f32>(input.m0, input.m1, input.m2, input.m3);
    let world_pos = model_matrix * vec4<f32>(input.position, 1.0);
    out.section_distance = dot(camera.section, world_pos);
    out.position = camera.view_projection * world_pos;
    let dim = input.inst_params.y;
    let profile = is_profile(input) && camera.style.w > 0.5;
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
    if !is_profile(input) { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.x += -camera.reserved.x * out.position.w;
    return out;
}

@vertex
fn vs_profile_right(input: EdgeInput) -> EdgeOutput {
    var out = edge_vertex(input);
    if !is_profile(input) { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.x += camera.reserved.x * out.position.w;
    return out;
}

@vertex
fn vs_profile_up(input: EdgeInput) -> EdgeOutput {
    var out = edge_vertex(input);
    if !is_profile(input) { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.y += camera.reserved.y * out.position.w;
    return out;
}

@vertex
fn vs_profile_down(input: EdgeInput) -> EdgeOutput {
    var out = edge_vertex(input);
    if !is_profile(input) { out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0); }
    out.position.y += -camera.reserved.y * out.position.w;
    return out;
}

@fragment
fn fs_edge(in: EdgeOutput) -> @location(0) vec4<f32> {
    if in.section_distance < -0.00001 { discard; }
    return vec4<f32>(in.color, 1.0);
}

struct GroundOutput { @builtin(position) position: vec4<f32>, @location(0) world: vec3<f32> };
@vertex fn vs_ground(@builtin(vertex_index) index:u32) -> GroundOutput {
    let corners=array<vec2<f32>,6>(vec2<f32>(0.,0.),vec2<f32>(1.,0.),vec2<f32>(1.,1.),vec2<f32>(0.,0.),vec2<f32>(1.,1.),vec2<f32>(0.,1.));
    let xy=mix(sun.ground.xy,sun.ground.zw,corners[index]);
    var out:GroundOutput;out.world=vec3<f32>(xy,0.);out.position=camera.view_projection*vec4<f32>(out.world,1.);return out;
}
@fragment fn fs_ground(in:GroundOutput) -> @location(0) vec4<f32> {
    if dot(camera.section,vec4<f32>(in.world,1.)) < -0.00001 {discard;}
    var visibility=sunlight(in.world);
    if sun.direction.z<=0.0 {visibility=0.0;}
    let intensity=mix(sun.parameters.y,sun.direction.w,visibility);
    return vec4<f32>(vec3<f32>(0.40,0.43,0.37)*intensity,1.);
}
