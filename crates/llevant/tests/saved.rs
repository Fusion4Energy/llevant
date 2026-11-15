//! Saved models: a model read back from its saved bytes answers exactly as
//! the one compiled, and bytes that are not a saved model are refused.

use llevant::Depth;
use llevant::{Error, GridMode, Line, Model, Options};

#[test]
fn saved_models_answer_as_compiled_ones() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models/universes.i"
    );
    let opts = Options::default().with_grid(GridMode::Always);

    let fresh = Model::load(path, &opts, None).unwrap();
    let mut bytes = Vec::new();
    fresh.save(&mut bytes).unwrap();
    let cached = Model::read(&mut bytes.as_slice()).unwrap();
    assert_eq!(cached.timings()[0].0, "saved model");

    let b = fresh.bounds(llevant::Scope::Problem);
    let mut r = 0x9e3779b97f4a7c15u64;
    let mut f = move || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        (r >> 11) as f64 / (1u64 << 53) as f64
    };
    let pts: Vec<[f64; 3]> = (0..20_000)
        .map(|_| [0, 1, 2].map(|i| b.lo[i] + f() * (b.hi[i] - b.lo[i])))
        .collect();
    let (x, y) = (
        fresh.classify(&pts, Depth::All),
        cached.classify(&pts, Depth::All),
    );
    assert_eq!(x.ids, y.ids);
    assert_eq!(x.labels, y.labels);
    let lines: Vec<Line> = pts
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| Line::between(p[0], p[1]))
        .filter(|l| l.t_max > 0.0)
        .collect();
    let (x, y) = (
        fresh.trace(&lines, Depth::All).unwrap(),
        cached.trace(&lines, Depth::All).unwrap(),
    );
    assert_eq!(x.offsets, y.offsets);
    assert_eq!((x.t0, x.t1, x.ids), (y.t0, y.t1, y.ids));
    assert_eq!(x.labels, y.labels);
    let (x, y) = (
        fresh.fingerprints(1e-9).unwrap(),
        cached.fingerprints(1e-9).unwrap(),
    );
    let digests = |f: &llevant::Fingerprints| {
        let cells = f.cells.iter().map(|c| (c.geometry, c.content));
        let units = f.units.iter().map(|u| (u.geometry, u.content));
        cells.chain(units).collect::<Vec<_>>()
    };
    assert_eq!(digests(&x), digests(&y));

    // Not a saved model, another format version, another build, cut short.
    let refused = |b: &[u8]| matches!(Model::read(&mut &b[..]), Err(Error::Format(_)));
    assert!(refused(b"c an MCNP input, not a model"));
    let mut other = bytes.clone();
    other[8] ^= 1;
    assert!(refused(&other));
    let mut other = bytes.clone();
    other[16] ^= 1;
    assert!(refused(&other));
    assert!(refused(&bytes[..bytes.len() - 1]));
    // Cut anywhere (at section boundaries too), or with bytes after it.
    for n in (0..bytes.len()).step_by(bytes.len() / 97 + 1) {
        assert!(refused(&bytes[..n]), "cut at {n} of {}", bytes.len());
    }
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(refused(&longer));
    // A bad section length.
    let mut other = bytes.clone();
    let at = 8 + 4 + 4 + llevant::ENGINE_HASH.len();
    other[at + 7] = 0xff;
    assert!(refused(&other));

    // The octrees, saved after the rest, are back: the same work.
    let again = Model::from_bytes(&bytes).unwrap();
    assert_eq!(again.stats().grid_bytes, fresh.stats().grid_bytes);
    assert!(fresh.stats().grid_units > 0);
    assert_eq!(again.stats().grid_units, fresh.stats().grid_units);
}
