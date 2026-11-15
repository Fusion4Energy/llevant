//! Random models, points and lines shared by the property tests.

#![allow(dead_code)]

use llevant::Line;

pub struct Rng(pub u64);

impl Rng {
    pub fn f(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
    pub fn int(&mut self, n: usize) -> usize {
        (self.f() * n as f64) as usize % n
    }
}

fn surface(r: &mut Rng) -> String {
    let c = |r: &mut Rng| r.range(-6.0, 6.0);
    match r.int(10) {
        0 => format!("px {:.4}", c(r)),
        1 => format!("pz {:.4}", c(r)),
        2 => format!("p {:.4} {:.4} {:.4} {:.4}", c(r), c(r), c(r) + 0.5, c(r)),
        3 => format!(
            "s {:.4} {:.4} {:.4} {:.4}",
            c(r),
            c(r),
            c(r),
            r.range(1.0, 6.0)
        ),
        4 => format!("c/z {:.4} {:.4} {:.4}", c(r), c(r), r.range(1.0, 5.0)),
        5 => format!("c/x {:.4} {:.4} {:.4}", c(r), c(r), r.range(1.0, 5.0)),
        6 => format!(
            "k/z {:.4} {:.4} {:.4} {:.4} {}",
            c(r),
            c(r),
            c(r),
            r.range(0.2, 2.0),
            [-1, 1][r.int(2)]
        ),
        7 => format!(
            "tz {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
            c(r),
            c(r),
            c(r),
            r.range(3.0, 5.0),
            r.range(0.5, 2.0),
            r.range(0.5, 2.0)
        ),
        8 => {
            // Rotated cylinder as GQ: |d|² - (d·u)² - R² about a random axis.
            let u = [r.range(-1.0, 1.0), r.range(-1.0, 1.0), r.range(0.1, 1.0)];
            let n = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
            let u = u.map(|x| x / n);
            let ctr = [c(r), c(r), c(r)];
            let rad = r.range(1.0, 4.0);
            let a = |i: usize, j: usize| {
                if i == j {
                    1.0 - u[i] * u[j]
                } else {
                    -u[i] * u[j]
                }
            };
            let ac: Vec<f64> = (0..3)
                .map(|i| (0..3).map(|j| a(i, j) * ctr[j]).sum())
                .collect();
            let k: f64 = (0..3).map(|i| ctr[i] * ac[i]).sum::<f64>() - rad * rad;
            format!(
                "gq {:.10} {:.10} {:.10} {:.10} {:.10} {:.10} {:.10} {:.10} {:.10} {:.10}",
                a(0, 0),
                a(1, 1),
                a(2, 2),
                2.0 * a(0, 1),
                2.0 * a(1, 2),
                2.0 * a(0, 2),
                -2.0 * ac[0],
                -2.0 * ac[1],
                -2.0 * ac[2],
                k
            )
        }
        _ => format!(
            "rpp {:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
            -3.0 + c(r) / 2.0,
            3.0 + c(r) / 2.0,
            -4.0,
            4.0,
            -2.0 + c(r) / 3.0,
            5.0
        ),
    }
}

/// A random expression over surfaces `first..first+n`.
fn expr(r: &mut Rng, first: usize, n: usize) -> String {
    let leaf = |r: &mut Rng| format!("{}{}", ["-", ""][r.int(2)], first + r.int(n));
    let and = |r: &mut Rng| {
        (0..1 + r.int(3))
            .map(|_| leaf(r))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut parts: Vec<String> = (0..1 + r.int(3)).map(|_| format!("({})", and(r))).collect();
    if r.int(4) == 0 {
        parts[0] = format!("#{}", parts[0]);
    }
    parts.join(" : ")
}

/// Level 0: random cells, one cell filled with a rotated universe of random
/// cells, the rest of the world, and the graveyard.
pub fn random_model(seed: u64) -> String {
    random_model_with(seed, false)
}

/// With `exclusive`, the random level-0 cells stay out of the filled
/// spheres, so most points and lines there reach the deeper levels.
pub fn random_model_with(seed: u64, exclusive: bool) -> String {
    let mut r = Rng(seed | 1);
    let outside = if exclusive { " 300 310" } else { "" };
    let mut cells = Vec::new();
    let mut surfs = Vec::new();
    for i in 0..10 {
        surfs.push(format!("{} {}", 100 + i, surface(&mut r)));
    }
    for i in 0..6 {
        surfs.push(format!("{} {}", 200 + i, surface(&mut r)));
    }
    let mut level0 = Vec::new();
    for i in 0..4 {
        let id = 1 + i;
        // Exclusive: also outside the earlier random cells and the TRCL cells.
        let earlier: String = if exclusive {
            (1..id).map(|j| format!(" #{j}")).collect::<String>() + " #52 #53"
        } else {
            String::new()
        };
        cells.push(format!(
            "{id} 1 -1.0 -999 ({}){outside}{earlier} imp:n=1",
            expr(&mut r, 100, 10)
        ));
        level0.push(id);
    }
    // Filled sphere.
    surfs.push(format!(
        "300 s {:.3} {:.3} {:.3} {:.3}",
        r.range(-3.0, 3.0),
        r.range(-3.0, 3.0),
        r.range(-3.0, 3.0),
        r.range(2.0, 5.0)
    ));
    let (a, b) = (r.range(0.0, 90.0), r.range(0.0, 90.0));
    cells.push(format!(
        "50 0 -300 *fill=7 ({:.3} {:.3} {:.3}  {a:.3} {:.3} 90  {:.3} {a:.3} 90  90 90 0) imp:n=1",
        r.range(-2.0, 2.0),
        r.range(-2.0, 2.0),
        r.range(-2.0, 2.0),
        90.0 - a,
        90.0 + a,
    ));
    let _ = b;
    level0.push(50);
    for i in 0..3 {
        cells.push(format!(
            "{} 2 -2.0 ({}) u=7 imp:n=1",
            71 + i,
            expr(&mut r, 200, 6)
        ));
    }
    cells.push("74 0 #71 #72 #73 u=7 imp:n=1".into());

    // A sphere filled with a lattice (rectangular or hexagonal, random
    // orientation) whose fill array mixes universes 7, 9 and the lattice's
    // own universe 8.
    surfs.push(format!(
        "310 s {:.3} {:.3} {:.3} {:.3}",
        r.range(-4.0, 4.0),
        r.range(-4.0, 4.0),
        r.range(-4.0, 4.0),
        r.range(2.0, 4.0)
    ));
    let th = r.range(0.0, 60.0);
    let element = if r.int(2) == 0 {
        surfs.push(format!(
            "320 rpp {:.3} {:.3} {:.3} {:.3} -1.1 1.1",
            -r.range(0.8, 1.5),
            r.range(0.8, 1.5),
            -r.range(0.8, 1.5),
            r.range(0.8, 1.5)
        ));
        ("-320".to_string(), 1)
    } else {
        let pitch = r.range(1.5, 3.0);
        for k in 0..3 {
            let a = (th + 60.0 * k as f64).to_radians();
            surfs.push(format!(
                "{} p {:.6} {:.6} 0 {:.4}",
                321 + 2 * k,
                a.cos(),
                a.sin(),
                pitch / 2.0
            ));
            surfs.push(format!(
                "{} p {:.6} {:.6} 0 {:.4}",
                322 + 2 * k,
                a.cos(),
                a.sin(),
                -pitch / 2.0
            ));
        }
        surfs.push("327 pz 1.2".into());
        surfs.push("328 pz -1.2".into());
        ("-321 322 -323 324 -325 326 -327 328".to_string(), 2)
    };
    let entries: Vec<&str> = (0..7 * 7 * 7).map(|_| ["7", "9", "8"][r.int(3)]).collect();
    let mut fill = String::from("fill=-3:3 -3:3 -3:3");
    for chunk in entries.chunks(14) {
        fill.push_str("\n     ");
        fill.push_str(&chunk.join(" "));
    }
    cells.push(format!(
        "81 3 -3.0 {} lat={} u=8 imp:n=1 {fill}",
        element.0, element.1
    ));
    cells.push(format!(
        "51 0 -310 fill=8 ({:.3} {:.3} {:.3}) imp:n=1",
        r.range(-0.5, 0.5),
        r.range(-0.5, 0.5),
        r.range(-0.5, 0.5)
    ));
    level0.push(51);
    cells.push(format!("91 1 -1.0 ({}) u=9 imp:n=1", expr(&mut r, 200, 6)));
    cells.push("92 0 #91 u=9 imp:n=1".into());

    // A cell moved by TRCL (rotated about z), and a copy of it with LIKE BUT.
    let c = r.range(0.0, 90.0);
    cells.push(format!(
        "52 1 -1.0 {} ({}){outside} *trcl=({:.3} {:.3} {:.3}  {c:.3} {:.3} 90  {:.3} {c:.3} 90  90 90 0) imp:n=1",
        if exclusive { "-998" } else { "-999" },
        expr(&mut r, 100, 10),
        r.range(-3.0, 3.0),
        r.range(-3.0, 3.0),
        r.range(-3.0, 3.0),
        90.0 - c,
        90.0 + c,
    ));
    cells.push(format!(
        "53 like 52 but trcl=({:.3} 0 0)",
        r.range(-3.0, 3.0)
    ));
    level0.push(52);
    level0.push(53);
    let comp: Vec<String> = level0.iter().map(|c| format!("#{c}")).collect();
    cells.push(format!("98 0 -999 {} imp:n=1", comp.join(" ")));
    cells.push("99 0 999 imp:n=0".into());
    surfs.push("998 s 0 0 6 1.5".into());
    surfs.push("999 rpp -10 10 -10 10 -10 10".into());
    format!(
        "random\n{}\n\n{}\n\nm1 1001.32c 1\nm2 26056.32c 1\nm3 8016.32c 1\n",
        cells.join("\n"),
        surfs.join("\n")
    )
}

pub fn points(seed: u64, n: usize) -> Vec<[f64; 3]> {
    let mut r = Rng(seed.wrapping_mul(0x9e3779b97f4a7c15) | 1);
    (0..n)
        .map(|_| [0, 1, 2].map(|_| r.range(-11.0, 11.0)))
        .collect()
}

pub fn lines(seed: u64, n: usize) -> Vec<Line> {
    let mut r = Rng(seed.wrapping_mul(0xd1b54a32d192ed03) | 1);
    (0..n)
        .map(|_| {
            let origin = [0, 1, 2].map(|_| r.range(-11.0, 11.0));
            // Most lines pass through the middle, where the filled cells are.
            let d = if r.int(3) > 0 {
                let aim = [0, 1, 2].map(|_| r.range(-4.0, 4.0));
                [0, 1, 2].map(|i| aim[i] - origin[i])
            } else {
                [0, 1, 2].map(|_| r.range(-1.0, 1.0))
            };
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            Line {
                origin,
                dir: d.map(|x| x / len),
                t_min: r.range(-5.0, 0.0),
                t_max: r.range(5.0, 30.0),
            }
        })
        .collect()
}

/// `cases` property-test cases, unless `PROPTEST_CASES` asks for another
/// number (the nightly run asks for more).
pub fn proptest_config(cases: u32) -> proptest::test_runner::Config {
    if std::env::var_os("PROPTEST_CASES").is_some() {
        proptest::test_runner::Config::default()
    } else {
        proptest::test_runner::Config::with_cases(cases)
    }
}
