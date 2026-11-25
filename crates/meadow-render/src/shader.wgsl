struct Globals {
    view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    sun: vec4<f32>,
    fog: vec4<f32>,      // rgb = colour, a = density
    params: vec4<f32>,   // x = ambient, y = highlight y (brush), z = 1 if the target isn't sRGB, w unused
};

@group(0) @binding(0) var<uniform> g: Globals;

struct VIn {
    @location(0) pos: vec4<i32>,
    @location(1) color: vec4<f32>,
    @location(2) origin: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
    @location(2) @interpolate(flat) normal: u32,
};

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

@vertex
fn vs_main(v: VIn) -> VOut {
    var o: VOut;
    let world = v.origin.xyz + vec3<f32>(v.pos.xyz) / 8.0;
    o.clip = g.view_proj * vec4<f32>(world, 1.0);
    o.color = vec4<f32>(srgb_to_linear(v.color.rgb), v.color.a);
    o.world = world;
    o.normal = u32(v.pos.w);
    return o;
}

fn normal_of(i: u32) -> vec3<f32> {
    switch i {
        case 0u: { return vec3<f32>(0.0, 1.0, 0.0); }
        case 1u: { return vec3<f32>(0.0, -1.0, 0.0); }
        case 2u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 3u: { return vec3<f32>(-1.0, 0.0, 0.0); }
        case 4u: { return vec3<f32>(0.0, 0.0, 1.0); }
        case 5u: { return vec3<f32>(0.0, 0.0, -1.0); }
        default: { return vec3<f32>(0.0, 1.0, 0.0); }
    }
}

fn shade(i: VOut) -> vec4<f32> {
    let n = normal_of(i.normal);
    var face = 1.0;
    // Minecraft-like face shading keeps blocky terrain readable from every angle.
    if i.normal == 1u { face = 0.5; }
    else if i.normal == 2u || i.normal == 3u { face = 0.78; }
    else if i.normal == 4u || i.normal == 5u { face = 0.66; }
    else if i.normal == 6u { face = 0.85; }
    let sun = max(dot(n, normalize(g.sun.xyz)), 0.0);
    let light = face * (g.params.x + (1.0 - g.params.x) * sun);
    var rgb = i.color.rgb * light;
    let dist = distance(i.world, g.camera.xyz);
    let fog = 1.0 - exp(-pow(dist * g.fog.a, 2.0));
    rgb = mix(rgb, srgb_to_linear(g.fog.rgb), clamp(fog, 0.0, 1.0));
    if g.params.z > 0.5 {
        // Plain Rgba8Unorm target (no sRGB view support): encode here.
        rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(rgb, i.color.a);
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    return shade(i);
}

@fragment
fn fs_water(i: VOut) -> @location(0) vec4<f32> {
    var c = shade(i);
    // Fresnel-ish: more opaque at grazing angles
    let view = normalize(g.camera.xyz - i.world);
    let f = 1.0 - abs(view.y);
    c.a = clamp(c.a + f * 0.35, 0.0, 0.95);
    return c;
}
