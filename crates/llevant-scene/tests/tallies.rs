//! Fields from MCNP tally files, on `tests/fixtures/tallies` (MCNP output, from aci-tallies):
//!
//! - meshtal: a pencil beam, where every voxel the beam crosses scores its
//!   chord over its volume; with the MCNP input, meshes take their exact
//!   FMESH geometry, `TR=` included;
//! - MCTAL: one ray along +x through slabs and a lattice (`probe.mcnp`),
//!   so every F4 bin is its chord over its volume: the field at each point
//!   must be the value of the bin holding it most closely;
//! - D1SUNED cuvmsh (`FMESH` with `out=cuv`): each voxel's flux from its
//!   cells', against the same mesh scored without `out=cuv` (`d1suned/`).
#![cfg(feature = "tallies")]

use std::path::PathBuf;

use llevant::{Model, Options};
use llevant_scene::field::{
    read, read_tallies, tally_field, Bin, Format, TallyChoice, TallyFile, Values,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/tallies")
        .join(name)
}

#[test]
fn pencil_beam_values_lie_along_the_beam() {
    let input = fixture("pencil_beam_input.mcnp");
    let m = Model::load(&input, &Options::default(), None).unwrap();
    let file = TallyFile::open(
        &fixture("pencil_beam_input.msht"),
        Some(&llevant_scene::field::read_input(&input).unwrap()),
    )
    .unwrap();
    for tally in [2004, 2014, 2024, 2034, 2044, 2054, 2064, 2074, 2084, 2094] {
        let choice = TallyChoice {
            tally: Some(tally),
            ..TallyChoice::default()
        };
        let f = tally_field(&m, &file, &choice).unwrap();
        let Values::Mesh(mesh) = &f.values else {
            panic!("tally {tally}: not a mesh")
        };
        // The beam: SDEF POS and VEC of the input.
        let pos = [-56.3597402157, -26.9768830971, -30.9828571187];
        let dir = [0.8151391459, 0.3668126157, 0.4483265303];
        let inside = m.classify_point([0.0; 3]);
        let mut scored = 0;
        for k in 0..20000 {
            let t = 0.01 * k as f64;
            let p = [0, 1, 2].map(|i| pos[i] + t * dir[i]);
            if let Some(v) = mesh.geometry.locate(p) {
                if mesh.values[v].is_finite() {
                    assert_eq!(f.value(&m, &inside, p), Some(mesh.values[v] as f64));
                    scored += 1;
                }
            }
        }
        assert!(
            scored > 100,
            "tally {tally}: {scored} points on the beam scored"
        );
        // Voxels the beam misses scored nothing: a value of 0 there (drawn
        // in the scale's low colour, not as no value).
        let empty = mesh.values.iter().position(|&v| v == 0.0).unwrap();
        let off = mesh.geometry.voxel_center(empty);
        assert_eq!(f.value(&m, &inside, off), Some(0.0), "tally {tally}");
        assert!(f.name.starts_with(&format!("tally {tally}")), "{}", f.name);
    }
    let read = |choice: &TallyChoice| {
        read_tallies(&m, &fixture("pencil_beam_input.msht"), Some(&input), choice)
    };
    let mut bad = TallyChoice {
        tally: Some(2004),
        energy: Bin::Index(99),
        ..TallyChoice::default()
    };
    assert!(read(&bad).is_err());
    bad.energy = Bin::Total;
    bad.set_bins("u=1").unwrap();
    assert!(read(&bad).unwrap_err().to_string().contains("no u bins"));
    let none = TallyChoice {
        tally: Some(1),
        ..TallyChoice::default()
    };
    assert!(read(&none).is_err());
}

fn probe() -> (Model, PathBuf) {
    let input = fixture("mctal/probe.mcnp");
    (
        Model::load(&input, &Options::default(), None).unwrap(),
        input,
    )
}

fn probe_tally(
    m: &Model,
    input: Option<&PathBuf>,
    choice: &TallyChoice,
) -> llevant::Result<llevant_scene::field::Field> {
    read_tallies(
        m,
        &fixture("mctal/probe.m"),
        input.map(|p| p.as_path()),
        choice,
    )
}

/// The field along the ray (`y = 0.3`, `z = 0.7`) at each `x`, against the
/// values MCNP scored: chord / volume of the bin holding each point (cells
/// 1, 2, 4: 20 cm in 8000 cm³; lattice 7 in cell 3 has elements 5 cm
/// long, each with 3 cm of cell 8 (300 cm³) and 2 cm of cell 9 (1700 cm³),
/// four of them on the ray).
fn check_along(m: &Model, input: Option<&PathBuf>, tally: u32, expect: &[(f64, Option<f64>)]) {
    let choice = TallyChoice {
        tally: Some(tally),
        ..TallyChoice::default()
    };
    let f = probe_tally(m, input, &choice).unwrap_or_else(|e| panic!("tally {tally}: {e}"));
    for &(x, want) in expect {
        let p = [x, 0.3, 0.7];
        let got = f.value(m, &m.classify_point(p), p);
        match (got, want) {
            (Some(g), Some(w)) => assert!(
                (g - w).abs() <= 1e-5 * w,
                "tally {tally} at x = {x}: {g}, MCNP says {w}"
            ),
            (g, w) => assert_eq!(g, w, "tally {tally} at x = {x}"),
        }
    }
}

#[test]
fn mctal_cell_tallies_lie_where_mcnp_scored_them() {
    let (m, input) = probe();
    let input = Some(&input);
    let slab = 20.0 / 8000.0;
    // `1 2 3 4`: cell 3 is filled, and holds all that is inside it.
    let plain = [
        (-60.0, None),
        (-30.0, Some(slab)),
        (-10.0, Some(slab)),
        (2.5, Some(slab)),
        (10.5, Some(slab)),
        (30.0, Some(slab)),
    ];
    check_along(&m, input, 4, &plain);
    // `(1 2) (3 4) 1 T`: groups of equal slabs score as one of them.
    check_along(&m, input, 14, &plain);
    // `8 9 (8 < 7[0:3 0:0 0:0] < 3)`: 8 alone is all four of its pieces,
    // each chain one.
    let (all8, all9, one8, one9) = (12.0 / 300.0, 8.0 / 1700.0, 3.0 / 300.0, 2.0 / 1700.0);
    check_along(
        &m,
        input,
        24,
        &[
            (-30.0, None),
            (0.5, Some(all9)),
            (2.5, Some(one8)),
            (7.5, Some(one8)),
            (12.5, Some(one8)),
            (17.5, Some(one8)),
        ],
    );
    // `(8 < 7[0:1 0:0 0:0] < 3) (8 9 < 7[2 0 0] < 3)`: two cells at the
    // first level are a bin each.
    check_along(
        &m,
        input,
        34,
        &[
            (0.5, None),
            (2.5, Some(one8)),
            (7.5, Some(one8)),
            (10.5, Some(one9)),
            (12.5, Some(one8)),
            (17.5, None),
        ],
    );
    // `(8 9 < 3) (8 < 7[1 0 0] 7[3 0 0] < 3) (8 < 3) T`: the chains through
    // an element hold their pieces of 8 more closely than `(8 < 3)`.
    check_along(
        &m,
        input,
        44,
        &[
            (-30.0, None),
            (0.5, Some(all9)),
            (2.5, Some(all8)),
            (7.5, Some(one8)),
            (12.5, Some(all8)),
            (17.5, Some(one8)),
        ],
    );
}

#[test]
fn mctal_bins_along_other_dimensions_are_chosen() {
    let (m, input) = probe();
    let input = Some(&input);
    // FM54 (1) (2) (1e-110): three multiplier bins, no Total.
    let mut choice = TallyChoice {
        tally: Some(54),
        ..TallyChoice::default()
    };
    let e = probe_tally(&m, input, &choice).unwrap_err().to_string();
    assert!(e.contains("m bin") && e.contains("3"), "{e}");
    choice.set_bins("m=1").unwrap();
    let f = probe_tally(&m, input, &choice).unwrap();
    // Plain unfilled cells: a field per cell, no unit (a multiplier).
    let Values::Cells(v) = &f.values else {
        panic!("{:?}", f.values)
    };
    let c1 = m.cell_index(1).unwrap() as usize;
    assert!((v[c1] as f64 - 2.0 * 20.0 / 8000.0).abs() < 1e-8);
    assert_eq!(f.unit, "");
    // 2.5e-113, a value (scored), though below what f32 holds.
    choice.set_bins("m=2").unwrap();
    let Values::Cells(v) = probe_tally(&m, input, &choice).unwrap().values else {
        panic!()
    };
    assert_eq!(v[c1], 0.0);
    // Energy bins: an index out of range.
    choice = TallyChoice {
        tally: Some(64),
        energy: Bin::Index(99),
        ..TallyChoice::default()
    };
    assert!(probe_tally(&m, input, &choice).is_err());
    // The first tally that can be drawn, with its unit.
    let f = probe_tally(&m, input, &TallyChoice::default()).unwrap();
    assert!(f.name.starts_with("tally 4 result (n)"), "{}", f.name);
    assert_eq!(f.unit, "1/cm²");
    assert!(matches!(f.values, Values::Instances(_)));
    // Surface tallies are not drawn.
    for t in [1, 2, 11] {
        let choice = TallyChoice {
            tally: Some(t),
            ..TallyChoice::default()
        };
        let e = probe_tally(&m, input, &choice).unwrap_err().to_string();
        assert!(e.contains("surface tallies"), "{e}");
    }
}

#[test]
fn mctal_without_the_input() {
    let (_, input) = probe();
    let text = std::fs::read_to_string(&input).unwrap();
    let m = Model::from_text(&text, &Options::default());
    // Plain cells: the file's numbers are enough.
    check_along(&m, None, 4, &[(-30.0, Some(20.0 / 8000.0)), (-60.0, None)]);
    // Chains: the file names only their first cell.
    let choice = TallyChoice {
        tally: Some(24),
        ..TallyChoice::default()
    };
    let e = probe_tally(&m, None, &choice).unwrap_err().to_string();
    assert!(e.contains("MCNP input"), "{e}");
}

/// Every way of writing levels in `levels.mcnp` gives a field per
/// instance (one plain tally a field per cell), with values in the tallies whose bins the ray went through
/// (it misses cell 8: only the bins with cell 9 scored).
#[test]
fn mctal_levels_of_every_kind() {
    let input = fixture("mctal/levels.mcnp");
    let m = Model::load(&input, &Options::default(), None).unwrap();
    let file = TallyFile::open(
        &fixture("mctal/levels.m"),
        Some(&llevant_scene::field::read_input(&input).unwrap()),
    )
    .unwrap();
    let ids = file.tallies();
    assert_eq!(ids, [4, 14, 24, 34, 44, 54, 64, 84, 94]);
    let mut scored = Vec::new();
    for t in ids {
        let choice = TallyChoice {
            tally: Some(t),
            ..TallyChoice::default()
        };
        let f = tally_field(&m, &file, &choice).unwrap_or_else(|e| panic!("tally {t}: {e}"));
        // F94 `1 (2 4) T` is plain cells, none filled.
        let per_cell = matches!(f.values, Values::Cells(_));
        assert_eq!(per_cell, t == 94, "tally {t}");
        // (Bins that scored nothing hold 0, a value.)
        if f.numbers().any(|x| x.is_finite() && x != 0.0) {
            scored.push(t);
        }
    }
    assert_eq!(scored, [4, 24, 44, 94]);
}

/// Detectors and TMESH tallies say they are not drawn; `+F6` and `*F4`
/// have their units.
#[test]
fn mctal_kinds_and_units() {
    let input = fixture("mctal/kcode.mcnp");
    let m = Model::load(&input, &Options::default(), None).unwrap();
    let read = |t: u32| {
        let choice = TallyChoice {
            tally: Some(t),
            ..TallyChoice::default()
        };
        read_tallies(&m, &fixture("mctal/kcode.m"), Some(&input), &choice)
    };
    for t in [5, 15] {
        let e = read(t).unwrap_err().to_string();
        assert!(e.contains("detectors"), "tally {t}: {e}");
    }
    assert_eq!(read(4).unwrap().unit, "1/cm²");
    assert_eq!(read(6).unwrap().unit, "MeV/g");
}

/// A file's summary: its tallies, what each is and the bins to choose
/// from; the default tally, and a tally without Totals read as a sum.
#[test]
fn summaries_and_sums() {
    let (m, input) = probe();
    let file = TallyFile::open(
        &fixture("mctal/probe.m"),
        Some(&llevant_scene::field::read_input(&input).unwrap()),
    )
    .unwrap();
    let s = file.summary();
    assert_eq!(s.default, Some(4));
    assert_eq!(s.tallies.len(), file.tallies().len());
    let t1 = s.tally(1).unwrap();
    assert!(t1.not_drawn.as_ref().unwrap().contains("surface"));
    // FM54 (1) (2) (1e-110): three multiplier bins, no Total.
    let t54 = s.tally(54).unwrap();
    assert!(t54.kind.starts_with("F4:"), "{}", t54.kind);
    let ax = t54.axis('m').unwrap();
    assert_eq!((ax.labels.len(), ax.total), (3, false));
    assert_eq!(ax.label(Bin::Index(1)), "m 1");
    assert_eq!(ax.label(Bin::Sum), "sum of all");
    let choice = t54.choice();
    assert_eq!(choice.other[3], Bin::Index(0));
    assert!(tally_field(&m, &file, &choice).is_ok());
    // The sum: 1 + 2 + 1e-110 times the flux.
    let mut sum = choice;
    sum.set_bins("m=sum").unwrap();
    let Values::Cells(v) = tally_field(&m, &file, &sum).unwrap().values else {
        panic!()
    };
    let c1 = m.cell_index(1).unwrap() as usize;
    assert!(
        (v[c1] as f64 - 3.0 * 20.0 / 8000.0).abs() < 1e-8,
        "{}",
        v[c1]
    );
    // Its relative error: the bins are one flux scaled, so errors in
    // quadrature give sqrt(1 + 4) / 3 of the flux's.
    sum.error = true;
    let Values::Cells(e) = tally_field(&m, &file, &sum).unwrap().values else {
        panic!()
    };
    let mut one = choice;
    one.error = true;
    let Values::Cells(e0) = tally_field(&m, &file, &one).unwrap().values else {
        panic!()
    };
    if e0[c1] > 0.0 {
        assert!((e[c1] / e0[c1] - 5f32.sqrt() / 3.0).abs() < 1e-4);
    }
    // Energy bins labelled by their bounds.
    let t64 = s.tally(64).unwrap();
    let e = t64.axis('e').unwrap();
    assert!(
        e.labels.iter().all(|l| l.ends_with("MeV")),
        "{:?}",
        e.labels
    );
    assert!(e.total);

    // Meshtal: energy and time axes, exact with the input.
    let input = fixture("pencil_beam_input.mcnp");
    let file = TallyFile::open(
        &fixture("pencil_beam_input.msht"),
        Some(&llevant_scene::field::read_input(&input).unwrap()),
    )
    .unwrap();
    let s = file.summary();
    assert_eq!(s.default, Some(2004));
    // But FMESH2104, whose TR= mirrors it, which a cylindrical mesh can't.
    for t in &s.tallies {
        let header = t.id == 2104;
        assert_eq!(t.wants_input, header, "{t:?}");
        assert!(t.cells > 1);
    }
    let without = TallyFile::open(&fixture("pencil_beam_input.msht"), None).unwrap();
    assert!(without.summary().tallies.iter().all(|t| t.wants_input));
}

/// MCTAL bins that only the input names (groups, here without it): the
/// summary says so, and the default tally is one with named cells.
#[test]
fn unnamed_bins_without_the_input() {
    let file = TallyFile::open(&fixture("mctal/probe.m"), None).unwrap();
    let s = file.summary();
    // `(1 2) (3 4) 1 T`: three bins without a number.
    let t14 = s.tally(14).unwrap();
    assert!(t14.wants_input, "{t14:?}");
    assert!(
        t14.notes[0].starts_with("3 of its 4 bins"),
        "{:?}",
        t14.notes
    );
    let t4 = s.tally(4).unwrap();
    assert!(!t4.wants_input && t4.notes.is_empty(), "{t4:?}");
    assert_eq!(s.default, Some(4));
}

/// Each cell bin of a tally: as MCNP names it, the cells isolating it
/// shows, whether those are all of it, and its value at the bins chosen.
#[test]
fn cell_bins_with_their_values() {
    let (m, input) = probe();
    let file = TallyFile::open(
        &fixture("mctal/probe.m"),
        Some(&llevant_scene::field::read_input(&input).unwrap()),
    )
    .unwrap();
    let s = file.summary();
    // `(8 9 < 3) (8 < 7[1 0 0] 7[3 0 0] < 3) (8 < 3) T`: MCNP makes a
    // bin of each cell and element of a chain's levels.
    let b = &s.tally(44).unwrap().bins;
    let labels: Vec<&str> = b.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "(8<3)",
            "(9<3)",
            "(8<7[1 0 0]<3)",
            "(8<7[3 0 0]<3)",
            "(8<3)",
            "T"
        ]
    );
    assert_eq!(b[2].cells, [8]);
    assert!(b.iter().all(|b| !b.whole_cells));
    assert!(b[5].total && b[5].cells.is_empty());
    // `(1 2) (3 4) 1 T`
    let b = &s.tally(14).unwrap().bins;
    assert_eq!(
        (b[0].cells.as_slice(), b[0].whole_cells),
        (&[1, 2][..], true)
    );
    assert_eq!(b[0].label, "(1 2)");
    // Surface tallies have none.
    assert!(s.tally(1).unwrap().bins.is_empty());
    // Values: chord over volume, as the field has them.
    let choice = s.tally(4).unwrap().choice();
    let (f, values) = llevant_scene::field::tally_field_and_bins(&m, &file, &choice).unwrap();
    let values = values.unwrap();
    assert_eq!(values.len(), 4);
    assert!((values[0].0 - 20.0 / 8000.0).abs() < 1e-8, "{values:?}");
    assert!(values[0].1 >= 0.0);
    let p = [-30.0, 0.3, 0.7];
    assert_eq!(
        f.value(&m, &m.classify_point(p), p),
        Some(values[0].0 as f32 as f64)
    );

    // Without the input: the file's numbers, `?` for groups.
    let file = TallyFile::open(&fixture("mctal/probe.m"), None).unwrap();
    let s = file.summary();
    let b = &s.tally(14).unwrap().bins;
    let labels: Vec<&str> = b.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["?", "?", "1", "?"]);
    assert_eq!(b[2].cells, [1]);
}

fn mesh_values(m: &Model, file: &TallyFile, choice: &TallyChoice) -> Vec<f32> {
    let Values::Mesh(mesh) = tally_field(m, file, choice).unwrap().values else {
        panic!("tally {:?}: not a mesh", choice.tally)
    };
    mesh.values
}

/// A cuvmsh is drawn as its voxels' fluxes: what the same mesh scores
/// without `out=cuv` (FMESH24, the twin of 14, from the same histories),
/// bin by bin, Total included; errors are only estimates.
#[test]
fn cuvmsh_voxel_fluxes_match_the_twin_mesh() {
    let input = fixture("d1suned/input");
    let m = Model::load(&input, &Options::default(), None).unwrap();
    let parsed = llevant_scene::field::read_input(&input).unwrap();
    let cuv = TallyFile::open(&fixture("d1suned/cuvmsh"), Some(&parsed)).unwrap();
    let twin = TallyFile::open(&fixture("d1suned/meshtal"), Some(&parsed)).unwrap();
    assert_eq!(cuv.format(), Format::Cuvmsh);
    assert_eq!(cuv.tallies(), vec![4, 14, 44]);
    for energy in [Bin::Total, Bin::Index(0), Bin::Index(2), Bin::Sum] {
        let choice = |tally| TallyChoice {
            tally: Some(tally),
            energy,
            ..TallyChoice::default()
        };
        let a = mesh_values(&m, &cuv, &choice(14));
        let b = mesh_values(&m, &twin, &choice(24));
        assert_eq!(a.len(), b.len());
        assert!(a.iter().any(|v| v.is_finite()));
        for (x, y) in a.iter().zip(&b) {
            assert!(
                x.is_nan() && y.is_nan() || (x - y).abs() <= 1e-5 * y.abs(),
                "{energy:?}: {x} {y}"
            );
        }
    }
    // Relative errors, and another mesh read after 14 (the one kept).
    let mut choice = TallyChoice {
        tally: Some(14),
        error: true,
        ..TallyChoice::default()
    };
    let f = tally_field(&m, &cuv, &choice).unwrap();
    assert_eq!(f.unit, "");
    choice.tally = Some(4);
    choice.error = false;
    let f = tally_field(&m, &cuv, &choice).unwrap();
    assert_eq!(f.unit, "1/cm²");
    assert!(f.name.starts_with("tally 4 result"), "{}", f.name);
    // Its default: the first mesh.
    let first = tally_field(&m, &cuv, &TallyChoice::default()).unwrap();
    assert!(first.name.starts_with("tally 4 "), "{}", first.name);
    // A mesh tally has no bins but energy and time.
    choice.set_bins("m=1").unwrap();
    let e = tally_field(&m, &cuv, &choice).unwrap_err().to_string();
    assert!(e.contains("no m bins"), "{e}");
}

/// A cuvmsh's summary: its meshes, their energy bins as the field has
/// them, and, without the input, meshes from the header (no `TR=`).
#[test]
fn cuvmsh_summary_and_meshinfo() {
    let input = fixture("d1suned/input");
    let parsed = llevant_scene::field::read_input(&input).unwrap();
    let file = TallyFile::open(&fixture("d1suned/cuvmsh"), Some(&parsed)).unwrap();
    let s = file.summary();
    assert_eq!(s.format, Format::Cuvmsh);
    assert_eq!(s.default, Some(4));
    assert_eq!(s.histories, Some(2e4));
    let t14 = s.tally(14).unwrap();
    assert_eq!(t14.kind, "FMESH14:neutron");
    assert_eq!(
        t14.comment.as_deref(),
        Some("CuV rectangular, 4 energy bins")
    );
    assert_eq!(t14.cells, 2 * 2 * 2);
    assert!(!t14.wants_input);
    let e = t14.axis('e').unwrap();
    assert_eq!((e.labels.len(), e.total), (4, true));
    assert_eq!(s.tally(4).unwrap().axis('e').unwrap().labels.len(), 20);
    // No EMESH: one bin, nothing to choose.
    assert!(s.tally(44).unwrap().axes.is_empty());
    assert!(t14.notes.iter().any(|n| n.contains("cell under voxel")));

    // Mesh 44 is under TR=1: only the input places it.
    let m = Model::load(&input, &Options::default(), None).unwrap();
    let placed = |file: &TallyFile| {
        let choice = TallyChoice {
            tally: Some(44),
            ..TallyChoice::default()
        };
        let Values::Mesh(mesh) = tally_field(&m, file, &choice).unwrap().values else {
            panic!()
        };
        mesh.geometry.voxel_center(0)
    };
    let without = TallyFile::open(&fixture("d1suned/cuvmsh"), None).unwrap();
    assert!(without.summary().tallies.iter().all(|t| t.wants_input));
    let (a, b) = (placed(&file), placed(&without));
    // TR1 0 0 -10, x' = (0.8, 0.6, 0), y' = (-0.6, 0.8, 0): voxel 0's centre
    // (-25, 0, 10) in the header's frame is (0.8·-25, 0.6·-25, 10 - 10) placed.
    let close = |p: [f64; 3], q: [f64; 3]| (0..3).all(|i| (p[i] - q[i]).abs() < 1e-6);
    assert!(close(b, [-25.0, 0.0, 10.0]), "{b:?}");
    assert!(close(a, [-20.0, -15.0, 0.0]), "{a:?}");

    // A meshinfo holds no values: the error says what to open.
    for e in [
        TallyFile::open(&fixture("d1suned/meshinfo"), None)
            .err()
            .unwrap(),
        read(
            &m,
            &fixture("d1suned/meshinfo"),
            None,
            &TallyChoice::default(),
        )
        .unwrap_err(),
    ] {
        assert!(e.to_string().contains("cuvmsh"), "{e}");
    }
}
