// ---- surfaces ------------------------------------------------------------

fn shape_kind(s: u32) -> u32 {
    return word(P.shape_info + s) >> 30u;
}

fn shape_base(s: u32) -> u32 {
    return word(P.shape_info + s) & MASK;
}

fn quadric_ad(b: u32, d: vec3<f32>) -> vec3<f32> {
    let a0 = sf(b + 3u);
    let a1 = sf(b + 4u);
    let a2 = sf(b + 5u);
    let a3 = sf(b + 6u);
    let a4 = sf(b + 7u);
    let a5 = sf(b + 8u);
    return vec3<f32>(
        a0 * d.x + a3 * d.y + a5 * d.z,
        a3 * d.x + a1 * d.y + a4 * d.z,
        a5 * d.x + a4 * d.y + a2 * d.z,
    );
}

fn quadric_f(b: u32, p: vec3<f32>) -> f32 {
    let d = p - sv3(b);
    return dot(d, quadric_ad(b, d)) + dot(sv3(b + 9u), d) + sf(b + 12u);
}

fn torus_rot(b: u32) -> mat3x3<f32> {
    return transpose(mat3x3<f32>(sv3(b), sv3(b + 3u), sv3(b + 6u)));
}

fn torus_f(b: u32, p: vec3<f32>) -> f32 {
    return torus_frame_f(b, torus_rot(b) * p + sv3(b + 9u));
}

// The torus function at `q`, already in the torus frame.
fn torus_frame_f(b: u32, q: vec3<f32>) -> f32 {
    let ax = u32(sf(b + 12u));
    let w = q[ax];
    let u = q[(ax + 1u) % 3u];
    let v = q[(ax + 2u) % 3u];
    let rho = sqrt(u * u + v * v);
    let dw = w / sf(b + 14u);
    let dr = (rho - sf(b + 13u)) / sf(b + 15u);
    return dw * dw + dr * dr - 1.0;
}

// Whether `p` is on the positive side of shape `s`.
fn side_pos(s: u32, p: vec3<f32>) -> bool {
    let b = shape_base(s);
    switch shape_kind(s) {
        case 0u: {
            return dot(sv3(b), p) - sf(b + 3u) > 0.0;
        }
        case 1u: {
            return quadric_f(b, p) > 0.0;
        }
        case 2u: {
            // Negative inside the double cone and on the nappe side.
            let neg = quadric_f(b, p) < 0.0 && dot(sv3(b + 16u), p - sv3(b + 13u)) > 0.0;
            return !neg;
        }
        default: {
            return torus_f(b, p) > 0.0;
        }
    }
}

// Gradient of shape `s` at `p` (not normalised).
fn gradient(s: u32, p: vec3<f32>) -> vec3<f32> {
    let b = shape_base(s);
    switch shape_kind(s) {
        case 0u: {
            return sv3(b);
        }
        case 1u, 2u: {
            return 2.0 * quadric_ad(b, p - sv3(b)) + sv3(b + 9u);
        }
        default: {
            let r = torus_rot(b);
            let q = r * p + sv3(b + 9u);
            let ax = u32(sf(b + 12u));
            let iu = (ax + 1u) % 3u;
            let iv = (ax + 2u) % 3u;
            let rho = sqrt(q[iu] * q[iu] + q[iv] * q[iv]);
            let a = sf(b + 13u);
            let bb = sf(b + 14u);
            let c = sf(b + 15u);
            var g = vec3<f32>(0.0);
            g[ax] = 2.0 * q[ax] / (bb * bb);
            if rho > 0.0 {
                let k = 2.0 * (rho - a) / (c * c * rho);
                g[iu] = k * q[iu];
                g[iv] = k * q[iv];
            }
            return transpose(r) * g;
        }
    }
}

// Roots of a s² + b s + c without cancellation (BIG when missing).
fn quad_roots(a: f32, b: f32, c: f32) -> vec2<f32> {
    if a == 0.0 {
        if b != 0.0 {
            return vec2<f32>(-c / b, BIG);
        }
        return vec2<f32>(BIG);
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return vec2<f32>(BIG);
    }
    let q = -0.5 * (b + select(1.0, -1.0, b < 0.0) * sqrt(disc));
    if q == 0.0 {
        return vec2<f32>(0.0, BIG);
    }
    return vec2<f32>(q / a, c / q);
}

// `t` if it lies in (lo, best), else `best`.
fn closer(t: f32, lo: f32, best: f32) -> f32 {
    return select(best, t, t > lo && t < best);
}

fn quadric_next(b: u32, o: vec3<f32>, d: vec3<f32>, lo: f32, best: f32) -> f32 {
    // Written about the vertex of the quadratic along the line (for a
    // cylinder, the point nearest its axis), kept in (lo, best) where the
    // roots wanted are: about the point nearest the quadric's origin, a ray
    // well away from that along a long cylinder would cancel badly. Where
    // the quadratic has no vertex, the point nearest the origin.
    let w = o - sv3(b);
    let ad = quadric_ad(b, d);
    let qa = dot(d, ad);
    let bv = sv3(b + 9u);
    var tc = -dot(w, d);
    if qa != 0.0 {
        tc = clamp(-(2.0 * dot(w, ad) + dot(bv, d)) / (2.0 * qa), lo, max(lo, best));
    }
    let e = w + tc * d;
    let qb = 2.0 * dot(e, ad) + dot(bv, d);
    let eae = dot(e, quadric_ad(b, e));
    let qc = eae + dot(bv, e) + sf(b + 12u);
    // Size of the terms that cancel in c, for the error estimate.
    let mc = abs(eae) + abs(dot(bv, e)) + abs(sf(b + 12u));
    let r = quad_roots(qa, qb, qc);
    var out = best;
    for (var k = 0u; k < 2u; k++) {
        let x = select(r.y, r.x, k == 0u);
        if x < BIG {
            let t = closer(tc + x, lo, out);
            if t < out {
                out = t;
                let slope = max(abs(2.0 * qa * x + qb), 1e-30);
                g_root_err = 3e-7 * ((abs(qa) * x * x + abs(qb * x) + mc) / slope + abs(t));
            }
        }
    }
    return out;
}

fn plane_next(n: vec3<f32>, off: f32, o: vec3<f32>, d: vec3<f32>, lo: f32, best: f32) -> f32 {
    let nd = dot(n, d);
    if nd == 0.0 {
        return best;
    }
    let t = closer((off - dot(n, o)) / nd, lo, best);
    if t < best {
        g_root_err = 3e-7 * ((abs(off) + dot(abs(n), abs(o))) / abs(nd) + abs(t));
    }
    return t;
}

fn poly(c: array<f32, 5>, x: f32) -> f32 {
    return (((c[4] * x + c[3]) * x + c[2]) * x + c[1]) * x + c[0];
}

// The root of `c` in [a, b], given a sign change there.
fn bisect(c: array<f32, 5>, a0: f32, b0: f32) -> f32 {
    var a = a0;
    var b = b0;
    let neg_a = poly(c, a) < 0.0;
    for (var i = 0u; i < 32u; i++) {
        let m = 0.5 * (a + b);
        if (poly(c, m) < 0.0) == neg_a {
            a = m;
        } else {
            b = m;
        }
    }
    return 0.5 * (a + b);
}

// Sign changes of `c` (degree ≤ 4) on [lo, hi], in increasing order: the
// roots of the derivative cut the range into monotone pieces. Fills `out`
// and returns how many.
fn cubic_roots(c: array<f32, 5>, lo: f32, hi: f32, out: ptr<function, array<f32, 4>>) -> u32 {
    // c is a cubic: its derivative's roots are closed form.
    var cuts = array<f32, 4>(lo, hi, hi, hi);
    var n = 1u;
    let r = quad_roots(3.0 * c[3], 2.0 * c[2], c[1]);
    let r0 = min(r.x, r.y);
    let r1 = max(r.x, r.y);
    if r0 > lo && r0 < hi {
        cuts[n] = r0;
        n++;
    }
    if r1 > lo && r1 < hi && r1 != r0 {
        cuts[n] = r1;
        n++;
    }
    cuts[n] = hi;
    var k = 0u;
    for (var i = 0u; i < n; i++) {
        let a = cuts[i];
        let b = cuts[i + 1u];
        let fa = poly(c, a);
        let fb = poly(c, b);
        if (fa < 0.0) != (fb < 0.0) {
            (*out)[k] = bisect(c, a, b);
            k++;
        }
    }
    return k;
}

// Smallest sign change of the quartic `c` in (lo, hi), or BIG.
fn quartic_first(c: array<f32, 5>, lo: f32, hi: f32) -> f32 {
    let dc = array<f32, 5>(c[1], 2.0 * c[2], 3.0 * c[3], 4.0 * c[4], 0.0);
    var crit = array<f32, 4>(hi, hi, hi, hi);
    let nc = cubic_roots(dc, lo, hi, &crit);
    var a = lo;
    var fa = poly(c, a);
    for (var i = 0u; i <= nc; i++) {
        let b = select(hi, crit[i], i < nc);
        let fb = poly(c, b);
        if (fa < 0.0) != (fb < 0.0) {
            return bisect(c, a, b);
        }
        a = b;
        fa = fb;
    }
    return BIG;
}

fn torus_next(b: u32, o: vec3<f32>, d: vec3<f32>, lo: f32, best: f32) -> f32 {
    let r = torus_rot(b);
    let q0 = r * o + sv3(b + 9u);
    let dq = r * d;
    let ax = u32(sf(b + 12u));
    let ta = sf(b + 13u);
    let tb = sf(b + 14u);
    let tcr = sf(b + 15u);
    // About the point closest to the centre, within the bounding sphere.
    let tc = -dot(q0, dq);
    let p = q0 + tc * dq;
    let rb = sqrt((ta + tcr) * (ta + tcr) + tb * tb) * 1.0001 + 1e-4;
    let h2 = dot(p, p);
    if h2 > rb * rb {
        return best;
    }
    let half = sqrt(rb * rb - h2);
    let slo = max(lo - tc, -half);
    let shi = min(best - tc, half);
    if !(slo <= shi) {
        return best;
    }
    let iu = (ax + 1u) % 3u;
    let iv = (ax + 2u) % 3u;
    let rr = vec3<f32>(
        p[iu] * p[iu] + p[iv] * p[iv],
        2.0 * (p[iu] * dq[iu] + p[iv] * dq[iv]),
        dq[iu] * dq[iu] + dq[iv] * dq[iv],
    );
    let ww = vec3<f32>(p[ax] * p[ax], 2.0 * p[ax] * dq[ax], dq[ax] * dq[ax]);
    let k = (tcr / tb) * (tcr / tb);
    let a2 = ta * ta;
    let g = vec3<f32>(rr.x + k * ww.x + a2 - tcr * tcr, rr.y + k * ww.y, rr.z + k * ww.z);
    if ta > 8.0 * max(tb, tcr) {
        return thin_torus_next(b, p, dq, rr, tc, slo, shi, lo, best);
    }
    var s = BIG;
    if ta == 0.0 {
        let q = quad_roots(g.z, g.y, g.x);
        s = closer(q.x, slo, min(shi, BIG));
        s = closer(q.y, slo, s);
        if s >= shi {
            s = BIG;
        }
    } else {
        let c = array<f32, 5>(
            g.x * g.x - 4.0 * a2 * rr.x,
            2.0 * g.y * g.x - 4.0 * a2 * rr.y,
            g.y * g.y + 2.0 * g.z * g.x - 4.0 * a2 * rr.z,
            2.0 * g.z * g.y,
            g.z * g.z,
        );
        s = quartic_first(c, slo, shi);
    }
    if s >= BIG {
        return best;
    }
    // The quartic's f32 coefficients cancel (a thin tube on a large ring
    // loses ~5e-3 cm): refine on the torus function itself, which keeps
    // ~1e-5 cm, when it changes sign near the root found.
    let w = max(0.02, 1e-5 * rb);
    var x0 = s - w;
    var x1 = s + w;
    let neg0 = torus_frame_f(b, p + x0 * dq) < 0.0;
    if neg0 != (torus_frame_f(b, p + x1 * dq) < 0.0) {
        for (var k = 0u; k < 24u; k++) {
            let m = 0.5 * (x0 + x1);
            if (torus_frame_f(b, p + m * dq) < 0.0) == neg0 {
                x0 = m;
            } else {
                x1 = m;
            }
        }
        s = 0.5 * (x0 + x1);
    }
    let t = closer(tc + s, lo, best);
    if t < best {
        g_root_err = 3e-7 * (rb + abs(t));
    }
    return t;
}

// First sign change of the torus function on [x0, x1] (in the torus frame,
// along `p + x·dq`), where it is unimodal; BIG if none.
fn torus_piece_root(b: u32, p: vec3<f32>, dq: vec3<f32>, x0: f32, x1: f32) -> f32 {
    let h0 = torus_frame_f(b, p + x0 * dq);
    let h1 = torus_frame_f(b, p + x1 * dq);
    var a = x0;
    var z = x1;
    if h0 >= 0.0 {
        // Outside at the start: the minimum must dip below zero.
        var u = x0;
        var v = x1;
        for (var k = 0u; k < 24u; k++) {
            let m1 = u + 0.381966 * (v - u);
            let m2 = v - 0.381966 * (v - u);
            if torus_frame_f(b, p + m1 * dq) < torus_frame_f(b, p + m2 * dq) {
                v = m2;
            } else {
                u = m1;
            }
        }
        let xm = 0.5 * (u + v);
        if torus_frame_f(b, p + xm * dq) >= 0.0 {
            return BIG;
        }
        z = xm;
    } else if h1 < 0.0 {
        return BIG;
    }
    let neg_a = torus_frame_f(b, p + a * dq) < 0.0;
    for (var k = 0u; k < 30u; k++) {
        let m = 0.5 * (a + z);
        if (torus_frame_f(b, p + m * dq) < 0.0) == neg_a {
            a = m;
        } else {
            z = m;
        }
    }
    return 0.5 * (a + z);
}

// Crossings of a thin torus (ring radius ≫ tube): the quartic's f32
// coefficients would cancel to nothing. The ray is clipped to the ring's
// enclosing shell (inside the outer cylinder, outside the inner one, within
// the slab of the tube's height), where the torus function along it has a
// single minimum per piece.
fn thin_torus_next(b: u32, p: vec3<f32>, dq: vec3<f32>, rr: vec3<f32>, tc: f32, slo: f32, shi: f32, lo: f32, best: f32) -> f32 {
    let ax = u32(sf(b + 12u));
    let ta = sf(b + 13u);
    let tb = sf(b + 14u);
    let tcr = sf(b + 15u);
    // ρ² = rr.x + rr.y s + rr.z s² against (a ± c)²; |w| ≤ b.
    var x0 = slo;
    var x1 = shi;
    let ro = quad_roots(rr.z, rr.y, rr.x - (ta + tcr) * (ta + tcr));
    if ro.x >= BIG || ro.y >= BIG {
        return best;
    }
    x0 = max(x0, min(ro.x, ro.y));
    x1 = min(x1, max(ro.x, ro.y));
    if dq[ax] != 0.0 {
        let w0 = (-tb - p[ax]) / dq[ax];
        let w1 = (tb - p[ax]) / dq[ax];
        x0 = max(x0, min(w0, w1));
        x1 = min(x1, max(w0, w1));
    } else if abs(p[ax]) > tb {
        return best;
    }
    if !(x0 < x1) {
        return best;
    }
    // Remove the inner cylinder's interior.
    let ri = quad_roots(rr.z, rr.y, rr.x - (ta - tcr) * (ta - tcr));
    var s = BIG;
    if ri.x < BIG && ri.y < BIG {
        let i0 = min(ri.x, ri.y);
        let i1 = max(ri.x, ri.y);
        if i0 > x0 {
            s = torus_piece_root(b, p, dq, x0, min(i0, x1));
        }
        if s >= BIG && i1 < x1 {
            s = torus_piece_root(b, p, dq, max(i1, x0), x1);
        }
    } else {
        s = torus_piece_root(b, p, dq, x0, x1);
    }
    if s >= BIG {
        return best;
    }
    let t = closer(tc + s, lo, best);
    if t < best {
        g_root_err = 3e-7 * (ta + tcr + abs(t));
    }
    return t;
}

// Smallest crossing of shape `s` along `o + t·d` in (lo, best), or `best`.
fn shape_next(s: u32, o: vec3<f32>, d: vec3<f32>, lo: f32, best: f32) -> f32 {
    let b = shape_base(s);
    switch shape_kind(s) {
        case 0u: {
            return plane_next(sv3(b), sf(b + 3u), o, d, lo, best);
        }
        case 1u: {
            return quadric_next(b, o, d, lo, best);
        }
        case 2u: {
            // The nappe side flips on the plane through the apex.
            let axis = sv3(b + 16u);
            let t = plane_next(axis, dot(axis, sv3(b + 13u)), o, d, lo, best);
            return quadric_next(b, o, d, lo, t);
        }
        default: {
            return torus_next(b, o, d, lo, best);
        }
    }
}
