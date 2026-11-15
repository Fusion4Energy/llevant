//! Sets of cells by rule, as frontends select and hide them: by cell,
//! universe or material number, the cells that hold a universe, what is
//! inside others, combined with and, or and not.
//!
//! The text form is what the viewer's search box, view files and (later)
//! the command line take:
//!
//! | Text | Cells |
//! |---|---|
//! | `c 12 40-55`, `cell 12` | those cell numbers (a bare number is a cell) |
//! | `u 20040`, `universe 1 2` | every cell of those universes |
//! | `m 5`, `mat 5 7` | cells of those materials that are not filled |
//! | `s 15`, `surface 15 20-25` | cells whose cards use those surfaces (any facet, `TRCL` copies, through `#n`) |
//! | `void` | unfilled cells without material (`m 0`) |
//! | `filled` | every filled cell (envelopes and lattices) |
//! | `fill 20040` | the cells that universe 20040 fills |
//! | `label "HNB - BSV"`, `$ HNB - BSV`, `$/ttl/BHC11` | cells with a label (an inline `$` comment of its card) containing that text, any case; unquoted, the text runs up to an operator (`&`, `\|`, `,`, `!`, parentheses, `and`, `or`), quoted it may hold them. Labels are read from the input ([`crate::labels`], [`ModelIndex::set_labels`]): without them the rule is an error |
//! | `in <q>` | `<q>` and everything inside its filled cells |
//! | `12 < 100[1 0 0] < 5`, `10 11 < 4` | MCNP's repeated-structure notation: cells 12 where they are inside 100, itself inside 5 (at any depth; lattice elements are read and ignored, a selection being of cells) |
//! | `all` | every cell |
//! | `shown`, `visible` | the cells a view shows ([`Query::resolve_with`]); in the viewer, those drawn in 3D: not hidden, at the level shown, and not wholly cut away by the section's cuts. Elsewhere an error |
//! | `a & b`, `a and b` | both |
//! | `a \| b`, `a or b`, `a, b` | either |
//! | `!a`, `not a`, `(...)` | all but; grouping |
//!
//! `<` binds tightest, then `not`, then `and`, then `or`. [`Query`]'s `Display` gives
//! the text back in this form, so it round-trips.

use std::fmt;

use llevant::{Error, Model, Result};

use crate::index::{CellSet, ModelIndex};

/// A rule selecting cells of a model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    Cells(Vec<i64>),
    Universes(Vec<i64>),
    Materials(Vec<i64>),
    /// The cells bounded by any of these MCNP surfaces.
    Surfaces(Vec<i64>),
    Void,
    Filled,
    /// The cells that hold any of these universes.
    Holders(Vec<i64>),
    Inside(Box<Query>),
    /// MCNP's repeated-structure notation, deepest level first: the cells
    /// of the first level that are inside a cell of the second, itself
    /// inside one of the third... (at any depth).
    Path(Vec<Vec<i64>>),
    /// The cells with a label (an inline `$` comment) containing this
    /// text, any case ([`crate::labels`]).
    Label(String),
    All,
    /// The cells a view shows ([`Query::resolve_with`]): in the viewer,
    /// those drawn in 3D, with some part left by its cuts.
    Shown,
    And(Box<Query>, Box<Query>),
    Or(Box<Query>, Box<Query>),
    Not(Box<Query>),
}

impl Query {
    pub fn cell(id: i64) -> Query {
        Query::Cells(vec![id])
    }

    pub fn universe(u: i64) -> Query {
        Query::Universes(vec![u])
    }

    pub fn material(id: i64) -> Query {
        Query::Materials(vec![id])
    }

    pub fn surface(id: i64) -> Query {
        Query::Surfaces(vec![id])
    }

    pub fn label(text: impl Into<String>) -> Query {
        Query::Label(text.into())
    }

    /// Whether the rule has a `label` term: it then needs the cells' labels,
    /// read from the input ([`ModelIndex::set_labels`]).
    pub fn uses_labels(&self) -> bool {
        match self {
            Query::Label(_) => true,
            Query::Inside(q) | Query::Not(q) => q.uses_labels(),
            Query::And(a, b) | Query::Or(a, b) => a.uses_labels() || b.uses_labels(),
            _ => false,
        }
    }

    /// Whether the rule has a `shown` term: it then depends on a view
    /// ([`Query::resolve_with`]).
    pub fn uses_shown(&self) -> bool {
        match self {
            Query::Shown => true,
            Query::Inside(q) | Query::Not(q) => q.uses_shown(),
            Query::And(a, b) | Query::Or(a, b) => a.uses_shown() || b.uses_shown(),
            _ => false,
        }
    }

    pub fn and(self, o: Query) -> Query {
        Query::And(Box::new(self), Box::new(o))
    }

    pub fn or(self, o: Query) -> Query {
        Query::Or(Box::new(self), Box::new(o))
    }

    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Query {
        Query::Not(Box::new(self))
    }

    /// `self` with `o` added (`a | b`), lists of the same kind merged.
    pub fn plus(self, o: Query) -> Query {
        match (self, o) {
            (Query::Cells(mut a), Query::Cells(b)) => {
                for x in b {
                    if !a.contains(&x) {
                        a.push(x);
                    }
                }
                Query::Cells(a)
            }
            (a, b) if a == b => a,
            (a, b) => a.or(b),
        }
    }

    /// `self` without `o` (`a & !b`); a cell removed from a list of cells
    /// leaves the list.
    pub fn minus(self, o: Query) -> Query {
        match (self, o) {
            (Query::Cells(a), Query::Cells(b)) => {
                Query::Cells(a.into_iter().filter(|x| !b.contains(x)).collect())
            }
            (a, b) => a.and(b.not()),
        }
    }

    /// The cells of `m` the rule selects. [`Error::NoCell`],
    /// [`Error::NoUniverse`] or [`Error::Invalid`] (materials) for the first
    /// number `m` does not have; [`Error::Invalid`] for `shown`, which needs
    /// a view ([`Query::resolve_with`]).
    pub fn resolve(&self, m: &Model, x: &ModelIndex) -> Result<CellSet> {
        self.resolve_with(m, x, None)
    }

    /// As [`Query::resolve`], `shown` being the cells a view shows (what
    /// `shown` selects); `None` where there is no view, and `shown` is then
    /// an error.
    pub fn resolve_with(
        &self,
        m: &Model,
        x: &ModelIndex,
        shown: Option<&CellSet>,
    ) -> Result<CellSet> {
        let n = x.cell_count();
        Ok(match self {
            Query::Cells(ids) => {
                let mut s = CellSet::empty(n);
                for &id in ids {
                    s.insert(m.cell_index(id).ok_or(Error::NoCell(id))?);
                }
                s
            }
            Query::Universes(us) => {
                let mut s = CellSet::empty(n);
                for &u in us {
                    let unit = x.unit(u).ok_or(Error::NoUniverse(u))?;
                    for &c in &m.units()[unit as usize].cells {
                        s.insert(c);
                    }
                }
                s
            }
            Query::Materials(ids) => {
                let mut s = CellSet::empty(n);
                for &id in ids {
                    let cells = x.material_cells(id);
                    if cells.is_empty() && id != 0 && m.material(id).is_none() {
                        return Err(Error::Invalid(format!("no material {id}")));
                    }
                    for &c in cells {
                        s.insert(c);
                    }
                }
                s
            }
            Query::Surfaces(ids) => {
                let cells = x.surface_cells(m, ids);
                if cells.is_empty() {
                    // Name the first number no cell uses.
                    let id = ids
                        .iter()
                        .find(|&&id| x.surface_cells(m, &[id]).is_empty())
                        .unwrap_or(&ids[0]);
                    return Err(Error::Invalid(format!("no cell uses surface {id}")));
                }
                CellSet::from_cells(n, cells)
            }
            Query::Void => CellSet::from_cells(n, x.material_cells(0).iter().copied()),
            Query::Filled => {
                CellSet::from_cells(n, (0..n as u32).filter(|&c| m.cell(c).fill.is_some()))
            }
            Query::Holders(us) => {
                let mut s = CellSet::empty(n);
                for &u in us {
                    let unit = x.unit(u).ok_or(Error::NoUniverse(u))?;
                    for &c in x.holders(unit) {
                        s.insert(c);
                    }
                }
                s
            }
            Query::Inside(q) => x.with_contents(m, &q.resolve_with(m, x, shown)?),
            Query::Path(levels) => {
                // From the top down: each level's cells inside the one above.
                let mut s: Option<CellSet> = None;
                for level in levels.iter().rev() {
                    let mut l = Query::Cells(level.clone()).resolve(m, x)?;
                    if let Some(above) = &s {
                        l.intersect(&x.with_contents(m, above));
                    }
                    s = Some(l);
                }
                let s = s.unwrap_or_else(|| CellSet::empty(n));
                if s.is_empty() {
                    return Err(Error::Invalid(format!(
                        "nothing at `{self}`: those cells are not inside one another"
                    )));
                }
                s
            }
            Query::Label(text) => {
                let labels = x.labels().ok_or_else(|| {
                    Error::Invalid(format!(
                        "`{self}`: label rules need the cells' labels, the `$` comments \
                         of the input, which are not read here"
                    ))
                })?;
                // Numbers the model lacks (a universe opened alone) were left
                // out when the labels were given.
                let mut s = CellSet::empty(n);
                for k in labels.matching_labels(text) {
                    for &c in x.label_cells(k) {
                        s.insert(c);
                    }
                }
                s
            }
            Query::All => CellSet::full(n),
            Query::Shown => match shown {
                Some(s) if s.universe_len() == n => s.clone(),
                _ => {
                    return Err(Error::Invalid(
                        "`shown` is what a view shows: it needs one (the viewer's 3D view)".into(),
                    ))
                }
            },
            Query::And(a, b) => {
                let mut s = a.resolve_with(m, x, shown)?;
                s.intersect(&b.resolve_with(m, x, shown)?);
                s
            }
            Query::Or(a, b) => {
                let mut s = a.resolve_with(m, x, shown)?;
                s.union(&b.resolve_with(m, x, shown)?);
                s
            }
            Query::Not(a) => {
                let mut s = a.resolve_with(m, x, shown)?;
                s.invert();
                s
            }
        })
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, 0)
    }
}

impl Query {
    /// `level`: 0 at the top or inside `|`, 1 inside `&`, 2 inside `!`
    /// and `in` (parentheses when lower than needed).
    fn write(&self, f: &mut fmt::Formatter<'_>, level: u8) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, key: &str, v: &[i64]| {
            f.write_str(key)?;
            for x in v {
                write!(f, " {x}")?;
            }
            Ok(())
        };
        match self {
            Query::Cells(v) => list(f, "c", v),
            Query::Universes(v) => list(f, "u", v),
            Query::Materials(v) => list(f, "m", v),
            Query::Surfaces(v) => list(f, "s", v),
            Query::Holders(v) => list(f, "fill", v),
            Query::Void => f.write_str("void"),
            Query::Filled => f.write_str("filled"),
            Query::All => f.write_str("all"),
            Query::Shown => f.write_str("shown"),
            Query::Label(text) => {
                f.write_str("label \"")?;
                for c in text.chars() {
                    if c == '"' || c == '\\' {
                        f.write_str("\\")?;
                    }
                    write!(f, "{c}")?;
                }
                f.write_str("\"")
            }
            Query::Path(levels) => {
                for (k, level) in levels.iter().enumerate() {
                    if k > 0 {
                        f.write_str(" < ")?;
                    }
                    let nums: Vec<String> = level.iter().map(|x| x.to_string()).collect();
                    f.write_str(&nums.join(" "))?;
                }
                Ok(())
            }
            Query::Inside(q) => {
                f.write_str("in ")?;
                q.write(f, 2)
            }
            Query::Not(q) => {
                f.write_str("!")?;
                q.write(f, 2)
            }
            Query::And(a, b) => {
                let paren = level > 1;
                if paren {
                    f.write_str("(")?;
                }
                a.write(f, 1)?;
                f.write_str(" & ")?;
                b.write(f, 1)?;
                if paren {
                    f.write_str(")")?;
                }
                Ok(())
            }
            Query::Or(a, b) => {
                let paren = level > 0;
                if paren {
                    f.write_str("(")?;
                }
                a.write(f, 0)?;
                f.write_str(" | ")?;
                b.write(f, 0)?;
                if paren {
                    f.write_str(")")?;
                }
                Ok(())
            }
        }
    }
}

impl std::str::FromStr for Query {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Query, String> {
        let tokens = lex(s)?;
        if tokens.is_empty() {
            return Err("empty query".into());
        }
        let mut p = Parser { t: &tokens, k: 0 };
        let q = p.or()?;
        match p.t.get(p.k) {
            None => Ok(q),
            Some(t) => Err(format!("unexpected `{}`", t.text())),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Num(i64),
    Range(i64, i64),
    And,
    Or,
    Not,
    Open,
    Close,
    /// `<`: inside, in MCNP's repeated-structure notation.
    Below,
    /// A lattice element, `[1 0 0]` (read and ignored).
    Element(String),
    /// The text of a label: quoted, or a word after `label` / `$`.
    Text(String),
}

impl Tok {
    fn text(&self) -> String {
        match self {
            Tok::Word(w) => w.clone(),
            Tok::Num(n) => n.to_string(),
            Tok::Range(a, b) => format!("{a}-{b}"),
            Tok::And => "&".into(),
            Tok::Or => "|".into(),
            Tok::Not => "!".into(),
            Tok::Open => "(".into(),
            Tok::Close => ")".into(),
            Tok::Below => "<".into(),
            Tok::Element(e) => format!("[{e}]"),
            Tok::Text(t) => format!("\"{t}\""),
        }
    }
}

fn lex(s: &str) -> std::result::Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&ch) = chars.peek() {
        // After `label` or `$`, a word that is not an operator is its text.
        let text_next = matches!(out.last(), Some(Tok::Word(w)) if keyword(w) == Some(Key::Label));
        match ch {
            c if c.is_whitespace() => {
                chars.next();
            }
            '"' => {
                chars.next();
                let mut t = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c) => t.push(c),
                            None => return Err("missing `\"`".into()),
                        },
                        Some(c) => t.push(c),
                        None => return Err("missing `\"`".into()),
                    }
                }
                out.push(Tok::Text(t));
            }
            '$' => {
                chars.next();
                out.push(Tok::Word("label".into()));
            }
            c if text_next && !"&|,!~()".contains(c) => {
                // Unquoted text: up to an operator (`&`, `|`, `and`...).
                let rest: String = chars.clone().collect();
                let text = bare_text(&rest);
                for _ in text.chars() {
                    chars.next();
                }
                out.push(Tok::Text(text.trim().to_string()));
            }
            '&' => {
                chars.next();
                out.push(Tok::And);
            }
            '|' | ',' => {
                chars.next();
                out.push(Tok::Or);
            }
            '!' | '~' => {
                chars.next();
                out.push(Tok::Not);
            }
            '(' => {
                chars.next();
                out.push(Tok::Open);
            }
            ')' => {
                chars.next();
                out.push(Tok::Close);
            }
            '=' => {
                // `u=20040` reads as `u 20040`.
                chars.next();
            }
            '<' => {
                chars.next();
                out.push(Tok::Below);
            }
            '[' => {
                chars.next();
                let mut e = String::new();
                loop {
                    match chars.next() {
                        Some(']') => break,
                        Some(c) => e.push(c),
                        None => return Err("missing `]`".into()),
                    }
                }
                out.push(Tok::Element(e.trim().to_string()));
            }
            _ => {
                let mut w = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || "&|,!~()=<[".contains(c) {
                        break;
                    }
                    w.push(c);
                    chars.next();
                }
                out.push(word(&w)?);
            }
        }
    }
    Ok(out)
}

/// The start of `rest` that is a label's unquoted text: up to an operator
/// character or the word `and` / `or`.
fn bare_text(rest: &str) -> &str {
    let end = rest
        .find(|c: char| "&|,!~()".contains(c))
        .unwrap_or(rest.len());
    let run = &rest[..end];
    let mut at = 0;
    for w in run.split_whitespace() {
        let k = at + run[at..].find(w).expect("a word of it");
        if k > 0 && (w.eq_ignore_ascii_case("and") || w.eq_ignore_ascii_case("or")) {
            return &run[..k];
        }
        at = k + w.len();
    }
    run
}

fn word(w: &str) -> std::result::Result<Tok, String> {
    let lower = w.to_ascii_lowercase();
    match lower.as_str() {
        "and" => return Ok(Tok::And),
        "or" => return Ok(Tok::Or),
        "not" => return Ok(Tok::Not),
        _ => {}
    }
    if let Ok(n) = w.parse::<i64>() {
        return Ok(Tok::Num(n));
    }
    // `40-55`, not `-5` (no negative numbers in MCNP cell, universe or
    // material numbers).
    if let Some((a, b)) = w.split_once('-') {
        if let (Ok(a), Ok(b)) = (a.parse::<i64>(), b.parse::<i64>()) {
            if a > b {
                return Err(format!("range `{w}` runs backwards"));
            }
            if b - a > 1_000_000 {
                return Err(format!("range `{w}` is too long"));
            }
            return Ok(Tok::Range(a, b));
        }
    }
    // `c12`, `m5`, `u20040` read as `c 12`...
    let split = lower.find(|c: char| c.is_ascii_digit());
    if let Some(k) = split.filter(|&k| k > 0) {
        if lower[k..].parse::<i64>().is_ok() && keyword(&lower[..k]).is_some() {
            return Ok(Tok::Word(format!("{} {}", &lower[..k], &lower[k..])));
        }
    }
    if keyword(&lower).is_some() {
        Ok(Tok::Word(lower))
    } else {
        Err(format!("unknown word `{w}`"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Key {
    Cells,
    Universes,
    Materials,
    Surfaces,
    Holders,
    Void,
    Filled,
    Inside,
    Label,
    All,
    Shown,
}

fn keyword(w: &str) -> Option<Key> {
    Some(match w {
        "c" | "cell" | "cells" => Key::Cells,
        "u" | "universe" | "universes" => Key::Universes,
        "m" | "mat" | "material" | "materials" => Key::Materials,
        "s" | "surf" | "surface" | "surfaces" => Key::Surfaces,
        "fill" | "fills" | "holders" => Key::Holders,
        "void" => Key::Void,
        "filled" => Key::Filled,
        "in" | "inside" => Key::Inside,
        "all" => Key::All,
        "shown" | "visible" => Key::Shown,
        "label" | "labels" => Key::Label,
        _ => return None,
    })
}

struct Parser<'a> {
    t: &'a [Tok],
    k: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.k)
    }

    fn or(&mut self) -> std::result::Result<Query, String> {
        let mut q = self.and()?;
        while self.peek() == Some(&Tok::Or) {
            self.k += 1;
            q = q.or(self.and()?);
        }
        Ok(q)
    }

    fn and(&mut self) -> std::result::Result<Query, String> {
        let mut q = self.unary()?;
        while self.peek() == Some(&Tok::And) {
            self.k += 1;
            q = q.and(self.unary()?);
        }
        Ok(q)
    }

    fn unary(&mut self) -> std::result::Result<Query, String> {
        match self.peek().cloned() {
            Some(Tok::Not) => {
                self.k += 1;
                Ok(self.unary()?.not())
            }
            Some(Tok::Open) => {
                self.k += 1;
                let q = self.or()?;
                if self.peek() != Some(&Tok::Close) {
                    return Err("missing `)`".into());
                }
                self.k += 1;
                Ok(q)
            }
            Some(Tok::Num(_) | Tok::Range(..)) => {
                let first = self.numbers()?;
                self.path(first)
            }
            Some(Tok::Word(w)) => {
                self.k += 1;
                // `c12` was lexed as one word with its number.
                let (w, first) = match w.split_once(' ') {
                    Some((a, b)) => (a.to_string(), Some(b.parse::<i64>().expect("lexed"))),
                    None => (w, None),
                };
                let key = keyword(&w).expect("lexed");
                let nums = |p: &mut Self| -> std::result::Result<Vec<i64>, String> {
                    let mut v: Vec<i64> = first.into_iter().collect();
                    if matches!(p.peek(), Some(Tok::Num(_) | Tok::Range(..))) {
                        v.extend(p.numbers()?);
                    }
                    if v.is_empty() {
                        return Err(format!("`{w}` needs numbers"));
                    }
                    Ok(v)
                };
                Ok(match key {
                    Key::Cells => {
                        let first = nums(self)?;
                        self.path(first)?
                    }
                    Key::Universes => Query::Universes(nums(self)?),
                    Key::Materials => Query::Materials(nums(self)?),
                    Key::Surfaces => Query::Surfaces(nums(self)?),
                    Key::Holders => Query::Holders(nums(self)?),
                    Key::Void => Query::Void,
                    Key::Filled => Query::Filled,
                    Key::All => Query::All,
                    Key::Shown => Query::Shown,
                    Key::Inside => Query::Inside(Box::new(self.unary()?)),
                    Key::Label => {
                        // `label5` was lexed as `label 5`.
                        let text = match (first, self.peek()) {
                            (Some(n), _) => n.to_string(),
                            (None, Some(Tok::Text(t))) => {
                                let t = t.clone();
                                self.k += 1;
                                t
                            }
                            _ => String::new(),
                        };
                        if text.trim().is_empty() {
                            return Err(format!("`{w}` needs a text (`label \"BSV\"`)"));
                        }
                        Query::Label(text)
                    }
                })
            }
            Some(t) => Err(format!("unexpected `{}`", t.text())),
            None => Err("query ends too soon".into()),
        }
    }

    /// Numbers and ranges, each number's lattice element skipped.
    fn numbers(&mut self) -> std::result::Result<Vec<i64>, String> {
        let mut v = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::Num(n)) => v.push(*n),
                Some(Tok::Range(a, b)) => v.extend(*a..=*b),
                Some(Tok::Element(_)) if !v.is_empty() => {}
                _ => break,
            }
            self.k += 1;
        }
        Ok(v)
    }

    /// The cells `first`, or, with `<` after them, the path they start.
    fn path(&mut self, first: Vec<i64>) -> std::result::Result<Query, String> {
        let mut levels = vec![first];
        while self.peek() == Some(&Tok::Below) {
            self.k += 1;
            let level = self.numbers()?;
            if level.is_empty() {
                return Err("`<` needs cell numbers after it".into());
            }
            levels.push(level);
        }
        Ok(if levels.len() == 1 {
            Query::Cells(levels.pop().expect("one"))
        } else {
            Query::Path(levels)
        })
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Query {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Query {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Query, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::tests_model::MODEL;
    use llevant::Options;

    fn q(s: &str) -> Query {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn cells(s: &str) -> Vec<i64> {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        q(s).resolve(&m, &x)
            .unwrap()
            .iter()
            .map(|c| m.cell_id(c))
            .collect()
    }

    #[test]
    fn text_parses_and_round_trips() {
        assert_eq!(q("c 12 40-42"), Query::Cells(vec![12, 40, 41, 42]));
        assert_eq!(q("12"), Query::cell(12));
        assert_eq!(q("u=20040"), Query::universe(20040));
        assert_eq!(q("m5"), Query::material(5));
        assert_eq!(q("surface 15"), Query::surface(15));
        assert_eq!(q("s 2-4"), Query::Surfaces(vec![2, 3, 4]));
        assert_eq!(q("s15"), Query::surface(15));
        assert_eq!(q("u 1 & m 3"), Query::universe(1).and(Query::material(3)));
        assert_eq!(
            q("not void and filled or all"),
            Query::Void.not().and(Query::Filled).or(Query::All)
        );
        for s in [
            "c 1 2",
            "u 1 & !m 3",
            "(u 1 | u 2) & m 3",
            "in (c 4 | c 1)",
            "!(void | filled)",
            "fill 1 | in c 4 & m 5",
            "s 1 2 & !u 1",
            "10 < 1",
            "30 31 < 20 < 12 < 4",
            "!10 < 1 & m 3",
            "in 12 < 1",
        ] {
            assert_eq!(q(s).to_string(), s, "{s}");
            assert_eq!(q(&q(s).to_string()), q(s));
        }
        assert_eq!(q("30 < 20[1 0 0] < 12 < 4"), q("30 < 20 < 12 < 4"));
        assert_eq!(q("c 10 < 1"), q("10 < 1"));
        assert_eq!(q("(30 < 20)"), q("30<20"));
        for bad in [
            "",
            "u",
            "c 3-1",
            "x 3",
            "(u 1",
            "u 1 &",
            "m 2 )",
            "10 <",
            "10 < u 1",
            "10[1 0 0",
            "inside 1 > 10",
        ] {
            assert!(bad.parse::<Query>().is_err(), "{bad}");
        }
    }

    #[test]
    fn queries_select_cells() {
        assert_eq!(cells("u 1"), [10, 11, 12]);
        assert_eq!(cells("m 3"), [10, 30]);
        assert_eq!(cells("void"), [2, 5, 31]);
        assert_eq!(cells("filled"), [1, 4, 12, 20]);
        assert_eq!(cells("fill 1"), [1, 4]);
        assert_eq!(cells("fill 2"), [20]);
        assert_eq!(cells("in c 12"), [12, 20, 30, 31]);
        assert_eq!(cells("m 3 & !u 2"), [10]);
        assert_eq!(cells("c 1 | c 2, c 5"), [1, 2, 5]);
        // Lattice faces, and the sphere cells 30 and 31 share.
        assert_eq!(cells("s 5"), [20]);
        assert_eq!(cells("s 10"), [30, 31]);
        assert_eq!(cells("s 1"), [1, 2]);
        // Paths: at any depth, lattice elements ignored.
        assert_eq!(cells("10 < 1"), [10]);
        assert_eq!(cells("10 11 < 4"), [10, 11]);
        assert_eq!(cells("30 < 20[2 0 0] < 12 < 1"), [30]);
        assert_eq!(cells("30 < 1"), [30]);
        assert_eq!(cells("10 30 < 20"), [30]);
    }

    /// Every instance's path as the model writes it (Copy cell path) is a
    /// rule selecting its cell.
    #[test]
    fn paths_the_model_writes_select_their_cell() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let inst = m.instances();
        assert!(inst.total() > 10);
        for id in 0..inst.total() {
            let path = m.instance_path(id).unwrap();
            let text = m.format_path(&path);
            let got = q(&text).resolve(&m, &x).unwrap();
            let c = path.last().unwrap().cell;
            assert_eq!(got.iter().collect::<Vec<_>>(), [c], "{text}");
        }
    }

    #[test]
    fn unknown_numbers_are_errors() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let err = |s: &str| q(s).resolve(&m, &x).unwrap_err().to_string();
        assert_eq!(err("c 7"), "no cell 7");
        assert_eq!(err("u 7"), "no universe 7");
        assert_eq!(err("m 7"), "no material 7");
        assert_eq!(err("fill 7"), "no universe 7");
        assert_eq!(err("s 77"), "no cell uses surface 77");
        assert_eq!(
            err("10 < 20"),
            "nothing at `10 < 20`: those cells are not inside one another"
        );
        assert_eq!(err("10 < 7"), "no cell 7");
    }

    #[test]
    fn labels_parse_and_round_trip() {
        assert_eq!(q("label BSV"), Query::label("BSV"));
        assert_eq!(q("$ BSV"), Query::label("BSV"));
        assert_eq!(q("$/ttl/BHC11"), Query::label("/ttl/BHC11"));
        assert_eq!(q("labels \"HNB - BSV\""), Query::label("HNB - BSV"));
        // Unquoted: up to an operator.
        assert_eq!(
            q("$ NB CELL - HNB - BSV"),
            Query::label("NB CELL - HNB - BSV")
        );
        assert_eq!(
            q("$ steel ball & m 3"),
            Query::label("steel ball").and(Query::material(3))
        );
        assert_eq!(
            q("label steel and m 3 or $ a b"),
            Query::label("steel")
                .and(Query::material(3))
                .or(Query::label("a b"))
        );
        assert_eq!(q("(label BSV)"), Query::label("BSV"));
        assert_eq!(q(r#"label "a \"b\" \\c""#), Query::label(r#"a "b" \c"#));
        assert_eq!(
            q("!label x & m 3"),
            Query::label("x").not().and(Query::material(3))
        );
        for s in [
            r#"label "HNB - BSV""#,
            r#"label "a \"b\" \\c""#,
            r#"!label "x" & m 3"#,
            r#"(label "a" | label "b") & u 1"#,
        ] {
            assert_eq!(q(s).to_string(), s);
            assert_eq!(q(&q(s).to_string()), q(s));
        }
        for bad in [r#"label "open"#, "label", "$", r#"label """#, "label (c 1)"] {
            assert!(bad.parse::<Query>().is_err(), "{bad}");
        }
        // A word that is not a keyword is still an error elsewhere.
        assert!("steel".parse::<Query>().is_err());
        assert!(q("u 1 & !(label x | c 4)").uses_labels());
        assert!(!q("u 1 & !(m 3 | c 4)").uses_labels());
    }

    #[test]
    fn shown_needs_a_view() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let q = |s: &str| s.parse::<Query>().unwrap();
        assert_eq!(q("shown"), Query::Shown);
        assert_eq!(q("visible & m 3"), Query::Shown.and(Query::material(3)));
        assert_eq!(q("!shown & m 3").to_string(), "!shown & m 3");
        assert!(q("u 1 & !(shown | c 4)").uses_shown());
        assert!(!q("u 1 & m 3").uses_shown());
        // No view: an error, not every cell or none.
        let err = q("shown & m 3").resolve(&m, &x).unwrap_err().to_string();
        assert!(err.contains("`shown`"), "{err}");
        // With one: what it shows, combined as any other term.
        let n = x.cell_count();
        let ten = m.cell_index(10).unwrap();
        let shown = CellSet::from_cells(n, [ten, m.cell_index(11).unwrap()]);
        let got = q("shown & m 3").resolve_with(&m, &x, Some(&shown)).unwrap();
        assert_eq!(got.iter().collect::<Vec<_>>(), [ten]);
        let all = q("!shown").resolve_with(&m, &x, Some(&shown)).unwrap();
        assert_eq!(all.count(), n - 2);
        // A set of another model's size is no view of this one.
        assert!(q("shown")
            .resolve_with(&m, &x, Some(&CellSet::empty(n + 1)))
            .is_err());
    }

    #[test]
    fn labels_resolve_once_given() {
        let text = MODEL
            .replace(
                "10 3 -7.8 -2 u=1 imp:n=1",
                "10 3 -7.8 -2 u=1 imp:n=1 $ Steel ball",
            )
            .replace(
                "11 5 -1.0 2 -4 u=1 imp:n=1",
                "11 5 -1.0 2 -4 u=1 imp:n=1 $ water",
            )
            .replace(
                "30 3 -7.8 -10 u=2 imp:n=1",
                "30 3 -7.8 -10 u=2 imp:n=1 $ steel pin",
            );
        let m = Model::from_text(&text, &Options::default());
        let x = ModelIndex::new(&m);
        let ids = |m: &Model, x: &ModelIndex, s: &str| -> Vec<i64> {
            q(s).resolve(m, x)
                .unwrap()
                .iter()
                .map(|c| m.cell_id(c))
                .collect()
        };
        // Not given: an error that says why.
        let err = q("label steel").resolve(&m, &x).unwrap_err().to_string();
        assert!(err.contains("not read"), "{err}");
        let labels = std::sync::Arc::new(crate::labels::Labels::of_text(&text));
        assert!(x.set_labels(&m, labels.clone()));
        assert!(!x.set_labels(&m, labels.clone()));
        assert_eq!(ids(&m, &x, "label STEEL"), [10, 30]);
        assert_eq!(ids(&m, &x, "$ steel & u 1"), [10]);
        assert_eq!(ids(&m, &x, "label water | label pin"), [11, 30]);
        assert!(ids(&m, &x, "label nothing").is_empty());
        // A universe on its own keeps the numbers: the same labels serve it.
        let u2 = m
            .universe(2, llevant::Cut::Own, Options::default().grid)
            .unwrap();
        let x2 = ModelIndex::new(&u2);
        x2.set_labels(&u2, labels);
        assert_eq!(ids(&u2, &x2, "label steel"), [30]);
    }

    #[test]
    fn plus_and_minus_merge_cell_lists() {
        assert_eq!(Query::cell(1).plus(Query::cell(2)), q("c 1 2"));
        assert_eq!(q("c 1 2 3").minus(Query::cell(2)), q("c 1 3"));
        assert_eq!(Query::universe(1).plus(Query::cell(2)), q("u 1 | c 2"));
        assert_eq!(Query::universe(1).minus(Query::cell(2)), q("u 1 & !c 2"));
    }
}
