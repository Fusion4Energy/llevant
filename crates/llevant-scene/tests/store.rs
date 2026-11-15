//! The cache of compiled models: a second load reads what the first
//! compiled (and answers the same), other options or another input miss
//! it, and a store with no directory always compiles.

use llevant::Depth;
use llevant::{GridMode, Model, Options};
use llevant_scene::store::Store;

fn from_cache(m: &Model) -> bool {
    m.timings().iter().any(|(step, _)| *step == "saved model")
}

#[test]
fn loads_hit_and_miss() {
    let dir = std::env::temp_dir().join(format!("llevant-store-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::at(&dir, 1 << 30);
    let models = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models"
    );
    let path = std::path::Path::new(models).join("universes.i");
    let opts = Options::default().with_grid(GridMode::Always);

    let fresh = store.load(&path, &opts, None).unwrap();
    assert!(!from_cache(&fresh));
    assert_eq!(fresh.timings()[0].0, "read");
    let cached = store.load(&path, &opts, None).unwrap();
    assert!(from_cache(&cached));
    let pts: Vec<[f64; 3]> = (0..2000)
        .map(|k| {
            let t = k as f64 / 2000.0;
            [200.0 * t - 100.0, 37.0 * (7.0 * t).sin(), 50.0 * t - 25.0]
        })
        .collect();
    assert_eq!(
        fresh.classify(&pts, Depth::All).ids,
        cached.classify(&pts, Depth::All).ids
    );

    // Other options, another input, no cache: compiled.
    assert!(!from_cache(
        &store.load(&path, &Options::default(), None).unwrap()
    ));
    let other = std::path::Path::new(models).join("periodic.i");
    assert!(!from_cache(&store.load(&other, &opts, None).unwrap()));
    assert!(!from_cache(
        &Store::none().load(&path, &opts, None).unwrap()
    ));

    // A budget of one byte keeps only the newest entry.
    let tight = Store::at(&dir, 1);
    tight
        .load(&path, &Options::default().with_tolerance(1e-7), None)
        .unwrap();
    let entries = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            let p = e.as_ref().unwrap().path();
            p.extension().is_some_and(|x| x == "llevant")
        })
        .count();
    assert_eq!(entries, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

fn saved(m: &Model) -> Vec<u8> {
    let mut b = Vec::new();
    m.save(&mut b).unwrap();
    b
}

/// An input changed since its last load is recompiled against the model
/// cached for it, and the result is exactly a compile of the new input.
#[test]
fn a_changed_input_recompiles_against_its_last_version() {
    let dir = std::env::temp_dir().join(format!("llevant-store-last-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::at(dir.join("cache"), 1 << 30);
    let src = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models/universes.i"
    );
    let old = std::fs::read_to_string(src).unwrap();
    let path = dir.join("model.i");
    std::fs::write(&path, &old).unwrap();
    let opts = Options::default();
    store.load(&path, &opts, None).unwrap();

    // Move one surface.
    let mut m = migjorn::Model::parse(&old);
    let id = m.surfaces().filter_map(|s| s.id()).next().unwrap();
    let c = m.surface(id).unwrap().coeffs().unwrap();
    let k = c.len() - 1;
    m.surface_mut(id).unwrap().set_coeff(k, c[k] + 0.5).unwrap();
    let new = m.to_source();
    std::fs::write(&path, &new).unwrap();

    let staged = store.load_staged(&path, &opts, None, None).unwrap();
    assert!(
        staged
            .model
            .timings()
            .iter()
            .any(|(s, _)| *s == "last version"),
        "{:?}",
        staged.model.timings()
    );
    let changed = staged.changed.clone().expect("recompiled");
    assert!(!changed.cells_changed.is_empty());
    let done = store.complete(staged, None).unwrap();
    let fresh = Model::compile(new.as_bytes(), &opts, None).unwrap();
    assert!(saved(&done) == saved(&fresh));
    // Stored: the next load reads it.
    assert!(from_cache(&store.load(&path, &opts, None).unwrap()));
    let _ = std::fs::remove_dir_all(&dir);
}
