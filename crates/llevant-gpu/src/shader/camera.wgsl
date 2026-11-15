// ---- camera images -------------------------------------------------------

@compute @workgroup_size(64)
fn render(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32, @builtin(num_workgroups) nw: vec3<u32>) {
    s_paused = false;
    let l = lane(wid.x + wid.y * nw.x, lid);
    if l.ok {
        render_ray(l);
    }
    slice_end(lid, l);
}

fn render_ray(l: Lane) {
    var r = l.index;
    // The part of the ray the cut keeps being traced (`Cutaway::kept`),
    // the plane of the cut it starts on (NONE: none) and where it ends;
    // none traced yet for a new ray.
    var part = 0u;
    var cap = NONE;
    var t1 = 0.0;
    var tracing = false;
    if !l.fresh {
        let saved = trace_load();
        r = saved.r;
        part = saved.extra & 0xffu;
        cap = select(saved.extra >> 8u, NONE, (saved.extra >> 8u) == 0xffu);
        t1 = saved.t_end;
        tracing = true;
    }
    let px = job_pixel(r);
    if px.x >= P.width || px.y >= P.height {
        finish_camera_ray(r);
        return;
    }
    let aspect = f32(P.width) / f32(P.height);
    let x = (2.0 * (f32(px.x) + 0.5) / f32(P.width) - 1.0) * aspect;
    let y = 1.0 - 2.0 * (f32(px.y) + 0.5) / f32(P.height);
    var d: vec3<f32>;
    // The ray's start, from the eye.
    var offset = vec3<f32>(0.0);
    if P.right.w > 0.0 {
        d = P.fwd.xyz;
        offset = P.right.w * (x * P.right.xyz + y * P.up.xyz);
    } else {
        let h = P.fwd.w;
        d = normalize(P.fwd.xyz + x * h * P.right.xyz + y * h * P.up.xyz);
    }
    let o = P.eye.xyz + offset;
    let i = px.y * P.width + px.x;
    let reach = P.eye.w;
    if tracing {
        ray_root(o, d);
    } else {
        reset_ghosts();
    }
    var t = 0.0;
    var left = slice_words();
    loop {
        if !tracing {
            // Past every region the ray is in at t, then up to where it
            // enters the next one.
            if part > 8u {
                break;
            }
            for (var sweep = 0u; sweep < 8u; sweep++) {
                var moved = false;
                for (var k = 0u; k < P.cut_regions; k++) {
                    let iv = region_interval(k, offset, d);
                    if iv.a <= t && iv.b > t {
                        t = iv.b;
                        cap = iv.exit;
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
            }
            if t >= reach {
                break;
            }
            t1 = reach;
            for (var k = 0u; k < P.cut_regions; k++) {
                let iv = region_interval(k, offset, d);
                if iv.a > t && iv.a < iv.b && iv.a < t1 {
                    t1 = iv.a;
                }
            }
            trace_begin(o, d, t, t1);
            tracing = true;
        }
        let w0 = g_work;
        if !trace_run(i, d, work_end(left)) {
            trace_save(r, part | (min(cap, 0xffu) << 8u), t1);
            return;
        }
        left -= min(left, g_work - w0);
        tracing = false;
        let b = HIT_WORDS * i;
        let status = hits[b + 3u] & 0xffu;
        if status != ST_MISS {
            // A part that starts on a cut face inside a visible cell hits
            // there: shaded by the face's normal, which faces the ray.
            if !STATS && cap != NONE && hits[b + 2u] == NONE && status != ST_STEPS {
                let n = P.cut[cap].xyz;
                hits[b + 4u] = bitcast<u32>(n.x);
                hits[b + 5u] = bitcast<u32>(n.y);
                hits[b + 6u] = bitcast<u32>(n.z);
            }
            write_ghost(i);
            finish_camera_ray(r);
            return;
        }
        t = t1;
        cap = NONE;
        part += 1u;
    }
    write_hit(i, reach, ST_MISS, 0u, 0u, 0u, NONE, d);
    write_ghost(i);
    finish_camera_ray(r);
}

// Where a ray is in a region of the cut: from `a` to `b` (none if a >= b),
// leaving it through plane `exit` of `cut` (NONE: at reach).
struct Interval {
    a: f32,
    b: f32,
    exit: u32,
}

// Where the ray `eye + offset + t·d` is in region `r` of the cut.
fn region_interval(r: u32, offset: vec3<f32>, d: vec3<f32>) -> Interval {
    var start = 0u;
    if r > 0u {
        start = P.cut_ends[(r - 1u) / 4u][(r - 1u) % 4u];
    }
    let end = P.cut_ends[r / 4u][r % 4u];
    var a = 0.0;
    var b = P.eye.w;
    var exit = NONE;
    for (var k = start; k < end; k++) {
        let c = P.cut[k];
        let s0 = c.w + dot(offset, c.xyz);
        let ds = dot(d, c.xyz);
        if ds == 0.0 {
            if s0 <= 0.0 {
                return Interval(1.0, 0.0, NONE);
            }
        } else {
            let tk = -s0 / ds;
            if ds > 0.0 {
                a = max(a, tk);
            } else if tk < b {
                b = tk;
                exit = k;
            }
        }
    }
    return Interval(a, b, exit);
}
