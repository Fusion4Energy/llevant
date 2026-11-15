// Base colours, as `llevant render`: gaps and problems as their patterns
// (`marked`: a black and grey checker, magenta stripes), the pattern of the
// gaps a ray went through half over what it sees beyond them, grey
// rays out of steps, background for misses, and each cell's palette colour
// lit one of two ways (params[0].x):
//
// 0: flat, shaded by the angle between the ray and the surface it enters
//    through (`llevant_render::colours::shade`);
// 1: physically based (realistic colours; `llevant_render::shading::surface`,
//    whose formulas these mirror in f32): GGX specular and Lambert diffuse
//    under a key and a fill light fixed to the view and a sky-to-ground
//    environment, with the cell's roughness and metalness, blended by its
//    clarity towards a glossy surface over a coloured depth (water),
//    tone-mapped (ACES) to sRGB; the background fades to light grey below
//    the view's horizon.

const PI: f32 = 3.14159265;
const KEY: f32 = 2.6;
const FILL: f32 = 0.7;
const EXPOSURE: f32 = 0.7;
const SKY: vec3<f32> = vec3<f32>(0.62, 0.66, 0.72);
const GROUND: vec3<f32> = vec3<f32>(0.22, 0.21, 0.20);
const WATER_F0: f32 = 0.02;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(x: vec3<f32>) -> vec3<f32> {
    let c = clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * c, c <= vec3<f32>(0.0031308));
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn environment(d: vec3<f32>, up: vec3<f32>, roughness: f32) -> vec3<f32> {
    let w = 0.15 + 0.85 * roughness;
    let t = clamp(0.5 + 0.5 * dot(d, up) / w, 0.0, 1.0);
    return mix(GROUND, SKY, t);
}

fn env_brdf(f0: vec3<f32>, roughness: f32, nov: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nov)) * r.x + r.y;
    let a = -1.04 * a004 + r.z;
    let b = 1.04 * a004 + r.w;
    return f0 * a + b;
}

fn direct(albedo: vec3<f32>, rough: f32, metal: f32, f0: vec3<f32>, n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, e: f32) -> vec3<f32> {
    let nol = dot(n, l);
    if nol <= 0.0 {
        return vec3<f32>(0.0);
    }
    let nov = max(dot(n, v), 1e-4);
    let h = normalize(v + l);
    let noh = max(dot(n, h), 0.0);
    let voh = max(dot(v, h), 0.0);
    let a = rough * rough;
    let a2 = a * a;
    let dd = noh * noh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * dd * dd);
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let g = (nol / (nol * (1.0 - k) + k)) * (nov / (nov * (1.0 - k) + k));
    let x = 1.0 - voh;
    let fw = x * x * x * x * x;
    let f = f0 + (vec3<f32>(1.0) - f0) * fw;
    let spec = d * g / (4.0 * nol * nov);
    return ((vec3<f32>(1.0) - f) * (1.0 - metal) * albedo / PI + f * spec) * e * nol;
}

fn key_light() -> vec3<f32> {
    return normalize(-0.45 * F.right.xyz + 0.65 * F.up.xyz - 0.6 * F.fwd.xyz);
}

fn fill_light() -> vec3<f32> {
    return normalize(0.7 * F.right.xyz + 0.05 * F.up.xyz - 0.7 * F.fwd.xyz);
}

fn pbr(albedo: vec3<f32>, roughness: f32, metalness: f32, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let u = F.up.xyz;
    let key = key_light();
    let fill = fill_light();
    let rough = clamp(roughness, 0.04, 1.0);
    let metal = clamp(metalness, 0.0, 1.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    var c = direct(albedo, rough, metal, f0, n, v, key, KEY) + direct(albedo, rough, metal, f0, n, v, fill, FILL);
    let nov = max(dot(n, v), 1e-4);
    let diffuse = environment(n, u, 1.0);
    let refl = n * (2.0 * dot(n, v)) - v;
    let spec_env = environment(refl, u, rough);
    let brdf = env_brdf(f0, rough, nov);
    c += (1.0 - metal) * albedo * diffuse + brdf * spec_env;
    return c;
}

// A clear surface (`llevant_render::shading::clear`).
fn clear(albedo: vec3<f32>, roughness: f32, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let u = F.up.xyz;
    let rough = clamp(roughness, 0.04, 1.0);
    let f0 = vec3<f32>(WATER_F0);
    let zero = vec3<f32>(0.0);
    var c = direct(zero, rough, 0.0, f0, n, v, key_light(), KEY) + direct(zero, rough, 0.0, f0, n, v, fill_light(), FILL);
    let nov = max(dot(n, v), 1e-4);
    let refl = n * (2.0 * dot(n, v)) - v;
    let spec_env = environment(refl, u, rough);
    let brdf = env_brdf(f0, rough, nov);
    let sky = mix(GROUND, SKY, 0.75);
    let body = albedo * (sky + 0.35 * KEY);
    c += (vec3<f32>(1.0) - brdf) * body + brdf * spec_env;
    return c;
}

fn surface(albedo: vec3<f32>, m: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    var c: vec3<f32>;
    if m.z >= 1.0 {
        c = clear(albedo, m.x, n, v);
    } else {
        c = pbr(albedo, m.x, m.y, n, v);
        if m.z > 0.0 {
            c = mix(c, clear(albedo, m.x, n, v), m.z);
        }
    }
    return linear_to_srgb(aces(EXPOSURE * c));
}

fn base_colour(p: vec2<i32>) -> vec4<f32> {
    let pbr_on = F.params[0].x > 0.5;
    let status = hit_status(p);
    let d = ray_dir(p);
    if status == ST_MISS {
        if pbr_on {
            let s = 0.82 + 0.18 * (0.5 + 0.5 * dot(d, F.up.xyz));
            return vec4<f32>(F.background.rgb * s, 1.0);
        }
        return F.background;
    }
    var base = vec4<f32>(1.0, 0.0, 1.0, 1.0);
    let cell = hit_cell(p);
    let n = hit_normal(p);
    if status == ST_INSIDE && cell != NONE {
        base = cell_colour(cell);
        if pbr_on {
            let m = cell_material(cell);
            let v = -d;
            let nn = select(n, v, all(n == vec3<f32>(0.0)));
            return vec4<f32>(surface(srgb_to_linear(base.rgb), m, nn, v), 1.0);
        }
    } else if status == ST_STEPS {
        // Gave up (previews stop rays early): neutral, not an error.
        base = vec4<f32>(0.6, 0.6, 0.6, 1.0);
    } else if status == ST_GAP {
        base = vec4<f32>(marked(true, p), 1.0);
    } else if status != ST_INSIDE || cell == NONE {
        base = vec4<f32>(marked(false, p), 1.0);
    }
    var s = 1.0;
    if any(n != vec3<f32>(0.0)) {
        s = 0.25 + 0.75 * abs(dot(n, d));
    }
    return vec4<f32>(base.rgb * s, 1.0);
}

// The base colour, with the pattern of the gaps (or overlaps) the ray went
// through half over it (params[0].y), as `llevant_render::colours::veiled`.
fn effect(p: vec2<i32>) -> vec4<f32> {
    let c = base_colour(p);
    let v = hit_veils(p);
    if !any(v) {
        return c;
    }
    return vec4<f32>(mix(c.rgb, marked(!v.y, p), F.params[0].y), c.a);
}
