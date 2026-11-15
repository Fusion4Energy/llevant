//! Fingerprints: invariant under renumbering and rewriting, sensitive to
//! every change, and local (a change only reaches the cells that contain it).

use std::path::PathBuf;

use llevant::{Fingerprint, Fingerprints, Model, Options, DEFAULT_PRECISION};
use migjorn::IdKind;

fn compile(text: &str) -> Model {
    let m = Model::from_text(text, &Options::default());
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    m
}

fn fps(m: &Model) -> Fingerprints {
    m.fingerprints(DEFAULT_PRECISION).unwrap()
}

/// Fingerprint of the cell with MCNP number `id`.
fn cell(m: &Model, f: &Fingerprints, id: i64) -> Fingerprint {
    f.cells[m.cell_index(id).unwrap() as usize]
}

fn zoo(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/mcnp-oracle/models")
        .join(format!("{name}.i"));
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn renumbering_everything_changes_nothing() {
    for name in ["surfaces", "universes", "periodic"] {
        let text = zoo(name);
        let src = migjorn::Model::parse(&text);
        let a = Model::from_migjorn(&src, &Options::default());
        assert!(a.diagnostics().is_empty(), "{name}: {:?}", a.diagnostics());
        let mut src = src;
        src.renumber(IdKind::Cell, |c| c + 50_000);
        // Macrobody facet references, TR numbers in FILL and TRCL and
        // universes in fill arrays included (migjorn 0.3.13 and later).
        src.renumber(IdKind::Surface, |s| 3 * s + 11);
        src.renumber(IdKind::Material, |m| m + 40);
        src.renumber(IdKind::Transform, |t| t + 700);
        src.renumber(IdKind::Universe, |u| if u == 0 { 0 } else { u + 1000 });
        let b = Model::from_migjorn(&src, &Options::default());
        assert!(b.diagnostics().is_empty(), "{name}: {:?}", b.diagnostics());
        assert_ne!(a.cells()[0].id, b.cells()[0].id);
        if name == "universes" {
            assert!(b.units().iter().filter(|u| u.universe > 1000).count() >= 3);
        }
        let (fa, fb) = (fps(&a), fps(&b));
        assert_eq!(a.cells().len(), b.cells().len());
        // Cards keep their order, so cell indices correspond.
        for i in 0..a.cells().len() {
            assert_eq!(a.cells()[i].id + 50_000, b.cells()[i].id);
            assert_eq!(fa.cells[i], fb.cells[i], "{name}: cell {}", a.cells()[i].id);
            let (ua, ub) = (a.cells()[i].unit as usize, b.cells()[i].unit as usize);
            assert_eq!(
                fa.units[ua],
                fb.units[ub],
                "{name}: universe of cell {}",
                a.cells()[i].id
            );
        }
        // Distinct cells are told apart.
        let mut g: Vec<_> = fa.cells.iter().map(|f| f.content).collect();
        g.sort_unstable();
        g.dedup();
        assert!(
            g.len() * 10 > a.cells().len() * 9,
            "{name}: {} distinct",
            g.len()
        );
    }
}

const BASE: &str = "\
base
1 1 -1.0 -1 imp:n=1
2 0 -2 fill=5 (1 0 0) imp:n=1
3 2 -7.8 1 2 -9 imp:n=1
9 0 9 imp:n=0
51 1 -1.0 -51 u=5 imp:n=1
52 0 51 u=5 imp:n=1

1 s -10 0 0 4
2 rpp 5 15 -5 5 -5 5
9 so 100
51 c/z 0 0 2

m1 1001.80c 2 8016.80c 1
m2 26056.80c 1
";

fn changed(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from}");
    text.replacen(from, to, 1)
}

/// The cells (by number) whose geometry and content digests differ between
/// the base model and `text`.
fn differences(text: &str) -> (Vec<i64>, Vec<i64>) {
    let (a, b) = (compile(BASE), compile(text));
    let (fa, fb) = (fps(&a), fps(&b));
    let mut geo = Vec::new();
    let mut con = Vec::new();
    for c in a.cells() {
        let (x, y) = (cell(&a, &fa, c.id), cell(&b, &fb, c.id));
        if x.geometry != y.geometry {
            geo.push(c.id);
        }
        if x.content != y.content {
            con.push(c.id);
        }
    }
    (geo, con)
}

#[test]
fn changes_reach_exactly_the_cells_that_contain_them() {
    // A sphere 1e-5 cm larger: the two cells bounded by it.
    let t = changed(BASE, "1 s -10 0 0 4", "1 s -10 0 0 4.00001");
    assert_eq!(differences(&t), (vec![1, 3], vec![1, 3]));
    // Inside a universe: the cell there, and the cell it fills.
    let t = changed(BASE, "51 c/z 0 0 2", "51 c/z 0 1e-6 2");
    assert_eq!(differences(&t), (vec![2, 51, 52], vec![2, 51, 52]));
    // The fill transform: only the filled cell.
    let t = changed(BASE, "fill=5 (1 0 0)", "fill=5 (1 0 1e-5)");
    assert_eq!(differences(&t), (vec![2], vec![2]));
    // A material's composition: content only, for every cell made of it
    // (also inside universes).
    let t = changed(BASE, "8016.80c 1", "8016.80c 1.001");
    assert_eq!(differences(&t), (vec![], vec![1, 2, 51]));
    // A density.
    let t = changed(BASE, "3 2 -7.8", "3 2 -7.9");
    assert_eq!(differences(&t), (vec![], vec![3]));
    // Outside the problem or not.
    let t = changed(BASE, "9 0 9 imp:n=0", "9 0 9 imp:n=1");
    assert_eq!(differences(&t), (vec![9], vec![9]));
    // Below the precision: nothing.
    let t = changed(BASE, "1 s -10 0 0 4", "1 s -10 0 0 4.0000000000001");
    assert_eq!(differences(&t), (vec![], vec![]));
}

#[test]
fn equivalent_inputs_hash_the_same() {
    // Every cell written differently but meaning the same: macrobody vs
    // planes (some with flipped normals and senses), S vs SPH, C/Z vs a
    // scaled and negated GQ, TRCL vs moved surfaces, inline transforms vs TR
    // cards, reordered expressions, a material with scaled and reordered
    // fractions, different numbers.
    let a = compile(
        "a
1 1 -1.0 -1 imp:n=1
2 0 -2 -4 fill=5 (1 0 0) imp:n=1
3 2 -7.8 1 #2 -9 imp:n=1
4 1 -1.0 -6 trcl=3 imp:n=1
9 0 9 imp:n=0
51 1 -1.0 -51 u=5 imp:n=1
52 0 51 u=5 imp:n=1

1 s -10 0 0 4
2 rpp 5 15 -5 5 -5 5
4 so 30
6 so 2
9 so 100
51 c/z 1 2 3

m1 1001.80c 2 8016.80c 1
m2 26056.80c 1
tr3 0 0 50
",
    );
    let b = compile(
        "b
1 7 -1.0 -10 imp:n=1
2 0 (-24 -21 -23 -22 -26 25) -40 fill=8 (12) imp:n=1
3 3 -7.8 -9 #2 10 imp:n=1
4 7 -1.0 -60 imp:n=1
9 0 9 imp:n=0
51 7 -1.0 81 u=8 imp:n=1
52 0 -81 u=8 imp:n=1

10 sph -10 0 0 4
21 py 5
22 pz 5
23 p 0 -1 0 5
24 px 15
25 pz -5
26 p -1 0 0 -5
40 so 30
60 s 0 0 50 2
9 so 100
81 gq -2 -2 0 0 0 0 4 8 0 8

m7 8016.80c 0.5 1001.80c 1.0
m3 26056.80c 7
tr12 1 0 0
",
    );
    let (fa, fb) = (fps(&a), fps(&b));
    for id in [1, 2, 3, 4, 9, 51, 52] {
        assert_eq!(cell(&a, &fa, id), cell(&b, &fb, id), "cell {id}");
    }
}

#[test]
fn card_order_does_not_matter() {
    let (head, rest) = BASE.split_once('\n').unwrap();
    let (cells, rest) = rest.split_once("\n\n").unwrap();
    let mut lines: Vec<&str> = cells.lines().collect();
    lines.reverse();
    let reordered = format!("{head}\n{}\n\n{rest}", lines.join("\n"));
    let (a, b) = (compile(BASE), compile(&reordered));
    let (fa, fb) = (fps(&a), fps(&b));
    for c in a.cells() {
        assert_eq!(cell(&a, &fa, c.id), cell(&b, &fb, c.id), "cell {}", c.id);
    }
    assert_eq!(fa.units[0], fb.units[0]);
}

#[test]
fn precision_and_threads() {
    let m = compile(&zoo("universes"));
    let f = fps(&m);
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| fps(&m));
    assert_eq!(f.cells, one.cells);
    assert_eq!(f.units, one.units);
    // Digests at another precision never match.
    let g = m.fingerprints(1e-6).unwrap();
    assert!(f.cells.iter().zip(&g.cells).all(|(a, b)| a != b));
}

/// `fingerprint_diff` tells content, geometry and renumbering apart.
#[test]
fn diff_names_each_kind_of_change() {
    use llevant::Change;
    let a = compile(
        "a\n1 1 -1.0 -1 imp:n=1\n2 2 -2.0 1 -2 imp:n=1\n3 0 2 -9 imp:n=1\n9 0 9 imp:n=0\n\n\
         1 so 1\n2 so 2\n9 so 100\n\nm1 1001.32c 1\nm2 26056.32c 1\n",
    );
    let b = compile(
        "b\n1 1 -1.5 -1 imp:n=1\n20 2 -2.0 1 -2 imp:n=1\n3 0 2 -9 imp:n=1\n9 0 9 imp:n=0\n\n\
         1 so 1\n2 so 2\n9 so 120\n\nm1 1001.32c 1\nm2 26056.32c 1\n",
    );
    let d = a.fingerprint_diff(&b, DEFAULT_PRECISION).unwrap();
    assert_eq!(d.cells.unchanged, 0);
    assert_eq!(
        d.cells.changes,
        [
            Change::Content(1),
            Change::Geometry(3),
            Change::Geometry(9),
            Change::Renumbered { from: 2, to: 20 },
        ]
    );
    assert_eq!(d.universes.changes, [Change::Geometry(0)]);
    assert!(d.to_string().contains("  cell 2: renumbered 20\n"), "{d}");
    assert!(a
        .fingerprint_diff(&a, DEFAULT_PRECISION)
        .unwrap()
        .is_empty());
}
