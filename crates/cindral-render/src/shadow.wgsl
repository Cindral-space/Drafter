struct Sun { matrix: mat4x4<f32>, direction: vec4<f32>, parameters: vec4<f32>, section: vec4<f32>, ground: vec4<f32> };
@group(0) @binding(0) var<uniform> sun: Sun;
@group(0) @binding(1) var materials: texture_2d_array<f32>;
@group(0) @binding(2) var material_sampler: sampler;
struct Output { @builtin(position) position: vec4<f32>, @location(0) distance: f32, @location(1) alpha: f32, @location(2) uv: vec2<f32>, @location(3) layer: f32 };
@vertex fn vs_shadow(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>, @location(3) material: f32,
 @location(4) m0: vec4<f32>, @location(5) m1: vec4<f32>, @location(6) m2: vec4<f32>, @location(7) m3: vec4<f32>,
 @location(12) inherited: vec4<f32>, @location(13) params: vec4<f32>) -> Output {
 var out: Output;
 let world=mat4x4<f32>(m0,m1,m2,m3)*vec4<f32>(position,1.0);
 out.position=sun.matrix*world;
 out.distance=select(dot(sun.section,world),0.0,material<0.0);
 let packed=u32(max(material,0.0)+0.5);
 out.layer=f32(packed/2u);
 if params.x>0.5 && packed%2u==0u && params.z>0.0 {out.layer=params.z;}
 let n=abs(normal);
 out.uv=position.xy;
 if n.x>=n.y && n.x>=n.z {out.uv=position.yz;} else if n.y>=n.x && n.y>=n.z {out.uv=position.xz;}
 out.alpha=color.a;
 if params.x>0.5 && (u32(max(material,0.0)+0.5)%2u)==0u {out.alpha*=inherited.a;}
 if dot(normal,normal)==0.0 {out.alpha=0.0;}
 return out;
}
@fragment fn fs_shadow(in:Output) {
    let layer=min(u32(in.layer+0.5),textureNumLayers(materials)-1u);
    let alpha=textureSampleLevel(materials,material_sampler,in.uv,layer,0.0).a*in.alpha;
    if in.distance < -0.00001 || alpha<0.999 {discard;}
}

@vertex fn vs_shadow_edge(@location(0) position:vec3<f32>, @location(1) color:vec3<f32>, @location(4) m0:vec4<f32>, @location(5) m1:vec4<f32>, @location(6) m2:vec4<f32>, @location(7) m3:vec4<f32>) -> Output {
    let world=mat4x4<f32>(m0,m1,m2,m3)*vec4<f32>(position,1.0);
    var out:Output;out.uv=vec2<f32>(0.0);out.layer=0.0;out.position=sun.matrix*world;out.distance=dot(sun.section,world);out.alpha=select(1.0,0.0,dot(color,color)==0.0);return out;
}
