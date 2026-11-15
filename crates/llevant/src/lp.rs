//! Small linear programs in three variables (Seidel's randomised algorithm),
//! used to bound cells by their planes.

/// Constraint `a·x ≤ b`.
pub type Constraint = ([f64; 3], f64);

/// Coordinates beyond this are treated as infinite.
pub const BIG: f64 = 1e8;

/// Minimise `c·x` subject to `cons` and `|x_k| ≤ BIG`. Returns the optimal
/// point, or `None` if the constraints are (numerically) infeasible.
pub fn minimize(cons: &[Constraint], c: [f64; 3]) -> Option<[f64; 3]> {
    SCRATCH.with_borrow_mut(|s| solve(3, cons, c, [(-BIG, BIG); 3], s))
}

thread_local! {
    /// Reduced constraints of the 2- and 1-variable subproblems, reused: the
    /// LP runs millions of times while boxes are computed on every thread, and
    /// allocating per violated constraint made that phase 3× slower on Windows.
    static SCRATCH: std::cell::RefCell<[Vec<Constraint>; 2]> = const {
        std::cell::RefCell::new([Vec::new(), Vec::new()])
    };
}

/// Shuffle constraints into a fixed pseudo-random order (keeps the expected
/// linear running time while staying deterministic).
pub fn shuffle(cons: &mut [Constraint]) {
    let mut s = 0x2545f4914f6cdd1du64;
    for i in (1..cons.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        cons.swap(i, (s % (i as u64 + 1)) as usize);
    }
}

/// `scratch[d - 2]` holds the subproblem built when a constraint is violated.
fn solve(
    d: usize,
    cons: &[Constraint],
    c: [f64; 3],
    bounds: [(f64, f64); 3],
    scratch: &mut [Vec<Constraint>],
) -> Option<[f64; 3]> {
    if d == 1 {
        let (mut lo, mut hi) = bounds[0];
        for &(a, b) in cons {
            let a = a[0];
            if a.abs() < 1e-14 {
                if b < -1e-9 * (1.0 + b.abs()) {
                    return None;
                }
            } else if a > 0.0 {
                hi = hi.min(b / a);
            } else {
                lo = lo.max(b / a);
            }
        }
        if lo > hi + 1e-9 * (1.0 + lo.abs()) {
            return None;
        }
        let x = if c[0] < 0.0 { hi } else { lo.min(hi) };
        return Some([x, 0.0, 0.0]);
    }
    let mut x = [0.0; 3];
    for k in 0..d {
        x[k] = if c[k] < 0.0 { bounds[k].1 } else { bounds[k].0 };
    }
    for i in 0..cons.len() {
        let (a, b) = cons[i];
        let ax: f64 = (0..d).map(|k| a[k] * x[k]).sum();
        let mag: f64 = (0..d).map(|k| (a[k] * x[k]).abs()).sum();
        if ax <= b + 1e-9 * (1.0 + b.abs() + mag) {
            continue;
        }
        // The optimum lies on a·x = b: eliminate the variable with the
        // largest coefficient and solve in one dimension less.
        let p = (0..d)
            .max_by(|&i, &j| a[i].abs().total_cmp(&a[j].abs()))
            .unwrap();
        if a[p].abs() < 1e-12 {
            // 0·x ≤ b: only infeasible if b is clearly negative.
            if b < -1e-9 * (1.0 + b.abs()) {
                return None;
            }
            continue;
        }
        let mut keep = [0; 2];
        let mut r = [0.0; 2];
        for (kk, k) in (0..d).filter(|&k| k != p).enumerate() {
            keep[kk] = k;
            r[kk] = a[k] / a[p];
        }
        let keep = &keep[..d - 1];
        let x0 = b / a[p];
        // x_p = x0 - Σ r_k x_k
        let (sub, deeper) = scratch[..d - 1].split_last_mut().unwrap();
        sub.clear();
        for &(aj, bj) in &cons[..i] {
            let mut na = [0.0; 3];
            for (kk, &k) in keep.iter().enumerate() {
                na[kk] = aj[k] - aj[p] * r[kk];
            }
            sub.push((na, bj - aj[p] * x0));
        }
        let mut up = [0.0; 3];
        let mut down = [0.0; 3];
        for kk in 0..keep.len() {
            up[kk] = -r[kk];
            down[kk] = r[kk];
        }
        sub.push((up, bounds[p].1 - x0));
        sub.push((down, x0 - bounds[p].0));
        let mut nc = [0.0; 3];
        let mut nb = [(0.0, 0.0); 3];
        for (kk, &k) in keep.iter().enumerate() {
            nc[kk] = c[k] - c[p] * r[kk];
            nb[kk] = bounds[k];
        }
        let y = solve(d - 1, sub, nc, nb, deeper)?;
        let mut xp = x0;
        for (kk, &k) in keep.iter().enumerate() {
            x[k] = y[kk];
            xp -= r[kk] * y[kk];
        }
        x[p] = xp;
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_cube() {
        // |x+y| ≤ √2, |x-y| ≤ √2, |z| ≤ 1: a square rotated 45°, so x ∈ [-√2, √2].
        let s = 2f64.sqrt();
        let mut cons = vec![
            ([1.0, 1.0, 0.0], s),
            ([-1.0, -1.0, 0.0], s),
            ([1.0, -1.0, 0.0], s),
            ([-1.0, 1.0, 0.0], s),
            ([0.0, 0.0, 1.0], 1.0),
            ([0.0, 0.0, -1.0], 1.0),
        ];
        shuffle(&mut cons);
        let x = minimize(&cons, [1.0, 0.0, 0.0]).unwrap();
        assert!((x[0] + s).abs() < 1e-9, "{x:?}");
        let x = minimize(&cons, [0.0, -1.0, 0.0]).unwrap();
        assert!((x[1] - s).abs() < 1e-9, "{x:?}");
        let x = minimize(&cons, [0.0, 0.0, -1.0]).unwrap();
        assert!((x[2] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn infeasible_and_unbounded() {
        let cons = vec![([1.0, 0.0, 0.0], -1.0), ([-1.0, 0.0, 0.0], -1.0)];
        assert!(minimize(&cons, [1.0, 0.0, 0.0]).is_none());
        let cons = vec![([1.0, 1.0, 1.0], 1.0)];
        let x = minimize(&cons, [1.0, 0.0, 0.0]).unwrap();
        assert!(x[0] <= -BIG * 0.99);
    }
}
