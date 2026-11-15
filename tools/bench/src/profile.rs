//! Optional CPU profiling of a benchmark section, with no perf_event
//! permissions needed (pprof samples from a timer signal in-process).
//!
//! `LLEVANT_PROFILE=dir`: each profiled section writes `dir/<name>.svg` (a
//! flamegraph) and prints the functions with the most samples. Timer
//! signals reach few of many busy threads, so profile with
//! `RAYON_NUM_THREADS=1` (or a few) for enough samples.

#[cfg(unix)]
use std::collections::HashMap;

#[cfg(unix)]
pub struct Profile {
    guard: Option<(pprof::ProfilerGuard<'static>, String)>,
}

/// Start profiling section `name` if `LLEVANT_PROFILE` is set.
#[cfg(unix)]
pub fn start(name: &str) -> Profile {
    let guard = std::env::var("LLEVANT_PROFILE").ok().map(|dir| {
        let g = pprof::ProfilerGuardBuilder::default()
            .frequency(997)
            .blocklist(&["libc", "libgcc", "pthread", "vdso"])
            .build()
            .expect("start profiler");
        (g, format!("{dir}/{name}"))
    });
    Profile { guard }
}

#[cfg(unix)]
impl Drop for Profile {
    fn drop(&mut self) {
        let Some((g, path)) = self.guard.take() else {
            return;
        };
        let report = match g.report().build() {
            Ok(r) => r,
            Err(e) => return eprintln!("profile {path}: {e}"),
        };
        if let Some(dir) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::File::create(format!("{path}.svg")) {
            Ok(f) => {
                if let Err(e) = report.flamegraph(f) {
                    eprintln!("flamegraph {path}: {e}");
                }
            }
            Err(e) => eprintln!("{path}.svg: {e}"),
        }
        // Self and inclusive sample counts per function.
        let mut own: HashMap<String, usize> = HashMap::new();
        let mut incl: HashMap<String, usize> = HashMap::new();
        let mut total = 0;
        for (frames, &n) in &report.data {
            let n = n.max(0) as usize;
            total += n;
            let names: Vec<String> = frames
                .frames
                .iter()
                .flat_map(|f| f.iter().map(|s| short(&s.name())))
                .collect();
            if let Some(first) = names.first() {
                *own.entry(first.clone()).or_default() += n;
            }
            let mut seen: Vec<&String> = Vec::new();
            // Inclusive counts only for this crate's functions.
            for name in names.iter().filter(|n| n.starts_with("llevant")) {
                if !seen.contains(&name) {
                    seen.push(name);
                    *incl.entry(name.clone()).or_default() += n;
                }
            }
        }
        let top = |m: &HashMap<String, usize>, what: &str| {
            let mut v: Vec<_> = m.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            println!("profile {path}: {total} samples, top by {what}:");
            for (name, n) in v.into_iter().take(25) {
                println!("  {:5.1}%  {name}", 100.0 * *n as f64 / total.max(1) as f64);
            }
        };
        top(&own, "self");
        top(&incl, "inclusive");
    }
}

#[cfg(unix)]
/// A demangled name without its hash and generic arguments:
/// `<llevant::surface::Shape>::side` → `llevant::surface::Shape::side`,
/// `<T as Trait>::f` → `T::f`.
fn short(name: &str) -> String {
    let mut out = String::new();
    // Per open bracket: whether it holds generic arguments (dropped) and, for a
    // qualified path, whether ` as Trait` has started (dropped).
    let mut stack: Vec<(bool, bool)> = Vec::new();
    let dropping = |st: &[(bool, bool)]| st.iter().any(|&(g, a)| g || a);
    let chars: Vec<char> = name.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '<' => {
                let prev = out.chars().last();
                let generic = !dropping(&stack)
                    && prev.is_some_and(|p| p.is_alphanumeric() || p == '_' || p == ':');
                if generic && out.ends_with("::") {
                    out.truncate(out.len() - 2);
                }
                stack.push((generic || dropping(&stack), false));
            }
            '>' => {
                stack.pop();
            }
            ' ' if !dropping(&stack) && name[i..].starts_with(" as ") => {
                if let Some(top) = stack.last_mut() {
                    top.1 = true;
                }
            }
            _ if !dropping(&stack) => out.push(ch),
            _ => {}
        }
        i += 1;
    }
    match out.rfind("::h") {
        Some(i) if out[i + 3..].chars().all(|c| c.is_ascii_hexdigit()) => out[..i].to_string(),
        _ => out,
    }
}

/// pprof samples from signals, so profiling is Unix-only.
#[cfg(not(unix))]
pub struct Profile;

/// Nothing to write, but `drop(prof)` marks where a section ends on every target.
#[cfg(not(unix))]
impl Drop for Profile {
    fn drop(&mut self) {}
}

#[cfg(not(unix))]
pub fn start(_name: &str) -> Profile {
    if std::env::var_os("LLEVANT_PROFILE").is_some() {
        eprintln!("LLEVANT_PROFILE is ignored: profiling needs a Unix target");
    }
    Profile
}
