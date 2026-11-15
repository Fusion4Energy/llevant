//! `LLEVANT_SOURCE_HASH`: a digest of the engine's source, version and
//! dependencies (`llevant::ENGINE_HASH`), carried by saved models
//! (`src/saved.rs`) so that a build that compiles differently never reads
//! another build's.
//!
//! The dependencies count through a `Cargo.lock`: the workspace's when built
//! in this repository, else the one packaged with the crate. The parser
//! (migjorn) is pinned exactly in `Cargo.toml`, so the version alone already
//! fixes what parses the input wherever the crate is built.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    let mut files = Vec::new();
    collect(Path::new("src"), &mut files);
    files.sort();
    let mut h = blake3::Hasher::new();
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    let lock = std::fs::read("../../Cargo.lock").or_else(|_| std::fs::read("Cargo.lock"));
    if let Ok(lock) = lock {
        h.update(&lock);
    }
    for f in files {
        h.update(f.to_string_lossy().as_bytes());
        h.update(&std::fs::read(&f).unwrap());
    }
    println!(
        "cargo:rustc-env=LLEVANT_SOURCE_HASH={}",
        h.finalize().to_hex()
    );
}

fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else {
            out.push(p);
        }
    }
}
