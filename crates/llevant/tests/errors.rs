//! Bad arguments give errors, not panics.

use llevant::Depth;
use llevant::{
    Aabb, Camera, Error, Line, Model, Options, Visibility, VolumeGrid, DEFAULT_PRECISION,
};

const SPHERE: &str = "\
sphere
1 1 -1.0 -1 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 so 1
9 so 10

m1 1001.32c 1
";

fn model() -> Model {
    Model::from_text(SPHERE, &Options::default())
}

#[test]
fn lines_must_be_finite_with_unit_directions() {
    let m = model();
    let good = Line::new([0.0; 3], [1.0, 0.0, 0.0], 0.0, 5.0);
    let long = Line {
        dir: [2.0, 0.0, 0.0],
        ..good
    };
    let endless = Line {
        t_max: f64::INFINITY,
        ..good
    };
    let reversed = Line { t_min: 6.0, ..good };
    let zero = Line::new([0.0; 3], [0.0; 3], 0.0, 1.0);
    for (k, bad) in [long, endless, reversed, zero].into_iter().enumerate() {
        let r = m.trace(&[good, bad], Depth::All);
        assert!(matches!(r, Err(Error::Line { index: 1, .. })), "{k}: {r:?}");
        assert!(llevant::reference::trace(&m, &[bad], Depth::All).is_err());
        assert!(m.trace_line(&bad).is_err());
        assert!(m.first_hits(&[bad], &Visibility::all(&m), None).is_err());
    }
    assert!(m.trace(&[good], Depth::All).is_ok());
    assert!(m.trace(&[], Depth::All).is_ok());
}

#[test]
fn visibility_must_fit_the_model() {
    let m = model();
    let mut vis = Visibility::all(&m);
    assert!(matches!(vis.hide(&m, &[1, 77]), Err(Error::NoCell(77))));
    assert!(matches!(
        vis.hide_universe(&m, 5),
        Err(Error::NoUniverse(5))
    ));
    vis.cells.pop();
    let ray = Line::new([-5.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.0, 10.0);
    assert!(matches!(
        m.first_hits(&[ray], &vis, None),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn degenerate_cameras_and_grids() {
    let m = model();
    let mut cam = Camera::fit(&m.bounds(llevant::Scope::Contents), [1.0, 0.0, 0.0], 8, 8);
    assert!(m.render(&cam, &Visibility::all(&m)).is_ok());
    cam.target = cam.eye;
    assert!(m.render(&cam, &Visibility::all(&m)).is_err());

    let region = Aabb {
        lo: [-2.0; 3],
        hi: [2.0; 3],
    };
    assert!(m.volumes(&VolumeGrid::new(region, 0.5), None).is_ok());
    for bad in [
        VolumeGrid::new(region, 0.0),
        VolumeGrid::new(region, f64::NAN),
        VolumeGrid::new(Aabb::EMPTY, 0.5),
        VolumeGrid::new(Aabb::INFINITE, 0.5),
        VolumeGrid {
            probes: 1,
            ..VolumeGrid::new(region, 0.5)
        },
    ] {
        assert!(
            matches!(m.volumes(&bad, None), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn fingerprint_precision_and_missing_files() {
    let m = model();
    assert!(m.fingerprints(DEFAULT_PRECISION).is_ok());
    for p in [0.0, -1e-9, 0.5, f64::NAN] {
        assert!(matches!(m.fingerprints(p), Err(Error::Invalid(_))), "{p}");
    }
    let e = Model::load("no/such/model.i", &Options::default(), None)
        .err()
        .unwrap();
    assert!(matches!(e, Error::Io { .. }));
    assert!(e.to_string().starts_with("no/such/model.i: "), "{e}");
}

#[test]
fn volumes_report_progress_and_stop_when_cancelled() {
    let m = model();
    let region = Aabb {
        lo: [-2.0; 3],
        hi: [2.0; 3],
    };
    let p = llevant::Progress::new();
    assert_eq!(p.fraction(), 0.0);
    let v = m.volumes(&VolumeGrid::new(region, 0.05), Some(&p)).unwrap();
    assert_eq!(p.fraction(), 1.0);
    assert_eq!(
        v.total(),
        m.volumes(&VolumeGrid::new(region, 0.05), None)
            .unwrap()
            .total()
    );
    let p = llevant::Progress::new();
    p.cancel();
    assert!(matches!(
        m.volumes(&VolumeGrid::new(region, 0.05), Some(&p)),
        Err(Error::Cancelled)
    ));
}

#[test]
fn loading_reports_its_steps_and_stops_when_cancelled() {
    let path = format!(
        "{}/../../tools/mcnp-oracle/models/universes.i",
        env!("CARGO_MANIFEST_DIR")
    );
    let opts = llevant::Options::default();
    let p = llevant::Progress::new();
    let m = llevant::Model::load(&path, &opts, Some(&p)).unwrap();
    assert!(!m.cells().is_empty());
    assert_eq!(p.fraction(), 1.0);
    assert_eq!(p.stage(), "octrees");
    let p = llevant::Progress::new();
    p.cancel();
    assert!(matches!(
        llevant::Model::load(&path, &opts, Some(&p)),
        Err(Error::Cancelled)
    ));
}
