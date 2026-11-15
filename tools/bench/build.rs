//! The build profile, for `report`: the directory under `target` that
//! `OUT_DIR` sits in (`release`, `dist`, `bench`...).

fn main() {
    let out = std::env::var("OUT_DIR").unwrap_or_default();
    let parts: Vec<&str> = out.split(['/', '\\']).collect();
    let profile = parts
        .iter()
        .rposition(|p| *p == "build")
        .and_then(|i| i.checked_sub(1))
        .map_or("unknown", |i| parts[i]);
    println!("cargo:rustc-env=LLEVANT_BENCH_PROFILE={profile}");
}
