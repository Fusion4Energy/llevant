//! Legends of fields: which values a colour bar marks ([`tick_values`]),
//! how their numbers are written ([`labels`]: one notation for the whole
//! bar), where everything goes ([`layout`]: the labels measured, those that
//! would overlap left out, the box sized to hold them) and the legend drawn
//! into an RGB image ([`rasterize`]), for images saved with their legend.
//!
//! The layout is in points and measures text in IBM Plex Mono
//! ([`MONO_FONT`], [`text_width`]); the viewer draws the same layout on
//! screen with the same font, so that a saved image shows the legend seen.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

use crate::colours::Rgb;
use crate::scale::Scale;

/// IBM Plex Mono (SIL Open Font License, `assets/LICENSE-IBM-Plex.txt`):
/// legends' text, here and in the viewer.
pub const MONO_FONT: &[u8] = include_bytes!("../assets/IBMPlexMono-Regular.ttf");

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(MONO_FONT).expect("the bundled font")
}

/// How a legend writes its numbers. Whichever, every label of a bar is in
/// the same notation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Numbers {
    /// Decimals (`0.5`, `25`, `1000`) when every number reads well so,
    /// else all with an exponent (`1e0`, `1e3`, `2.5e12`).
    #[default]
    Auto,
    /// Powers of ten: `10³`, `2.5·10¹²` (and `0`).
    Powers,
    /// Decimals, however small or large: `0.001`, `1000000000000`.
    Decimal,
}

/// Which values a bar marks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ticks {
    /// Its ends and a few round values between.
    #[default]
    Auto,
    /// Every decade of a log scale, twice as many round values on a
    /// linear one.
    Many,
    /// Exactly this many in all, ends included (2 or more), evenly spaced
    /// along the bar (in decades on a log scale): every one drawn and
    /// labelled, whatever the room.
    Count(u8),
    /// One at each edge between discrete bands ([`Scale::bands`]), ends
    /// included, exactly; without bands, as [`Ticks::Auto`].
    Edges,
}

impl Ticks {
    /// Exactly where asked, every one kept: an exact count, or the band
    /// edges of `scale` (when it has bands).
    pub fn exact(self, scale: &Scale) -> bool {
        match self {
            Ticks::Count(_) => true,
            Ticks::Edges => scale.bands >= 2,
            _ => false,
        }
    }
}

/// The ticks and numbers of a legend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickStyle {
    pub ticks: Ticks,
    pub numbers: Numbers,
    /// At most this many significant digits (1 to 6); none, up to 4 (each
    /// number with as few as write it to 0.05 %).
    pub digits: Option<u8>,
}

/// The values a bar of `scale` marks: its ends and round values between
/// (powers of ten on a log scale), none two closer than 2 % of the bar.
pub fn tick_values(scale: &Scale, ticks: Ticks) -> Vec<f64> {
    let (lo, hi) = (scale.lo, scale.hi);
    match ticks {
        Ticks::Count(n) => return even(scale, n.max(2) as usize),
        Ticks::Edges if scale.bands >= 2 => return even(scale, scale.bands as usize + 1),
        _ => {}
    }
    let target: usize = match ticks {
        Ticks::Many => 0,
        _ => 6,
    };
    let inner: Vec<f64> = if scale.log {
        let (a, b) = (lo.log10().ceil() as i32, hi.log10().floor() as i32);
        let decades = (b - a + 1).max(0) as usize;
        let step = match ticks {
            Ticks::Many => 1,
            _ => decades.div_ceil(target.saturating_sub(1).max(1)).max(1),
        };
        (a..=b).step_by(step).map(|e| 10f64.powi(e)).collect()
    } else {
        let parts = match ticks {
            Ticks::Many => 8,
            _ => target.saturating_sub(1).max(1),
        };
        let step = nice_step((hi - lo) / parts as f64);
        let (first, last) = ((lo / step).ceil() as i64, (hi / step).floor() as i64);
        if last - first > 1000 {
            Vec::new()
        } else {
            (first..=last).map(|k| k as f64 * step).collect()
        }
    };
    let mut out = vec![lo];
    let at = |v: f64| scale.fraction(v);
    for v in inner {
        let t = at(v);
        // Not on top of an end, nor of another.
        if t > 0.02 && t < 0.98 && out.iter().all(|&k| (at(k) - t).abs() > 0.02) {
            out.push(v);
        }
    }
    if hi > lo {
        out.push(hi);
    }
    out
}

/// `n` values evenly spaced along `scale` (in decades on a log scale), its
/// ends included: those of [`Ticks::Count`] and of the band edges, which
/// [`Scale::banded`] puts at the same fractions. A value within a hair of a
/// power of ten (log) or a round number is that number.
fn even(scale: &Scale, n: usize) -> Vec<f64> {
    if !(scale.hi > scale.lo) {
        return vec![scale.lo];
    }
    (0..n)
        .map(|k| {
            let t = k as f64 / (n - 1) as f64;
            let v = scale.value(t);
            if scale.log {
                let e = v.log10();
                if (e - e.round()).abs() < 1e-9 {
                    return 10f64.powi(e.round() as i32);
                }
            }
            if v == 0.0 || !v.is_finite() {
                return v;
            }
            let q = 10f64.powi(11 - v.abs().log10().floor() as i32);
            if q.is_finite() && q > 0.0 {
                (v * q).round() / q
            } else {
                v
            }
        })
        .collect()
}

/// Whether the band edges of `scale` fall on powers of ten (a log scale
/// from one to another, each band a whole number of decades); `None`
/// without bands or on a linear scale.
pub fn edges_on_decades(scale: &Scale) -> Option<bool> {
    if !scale.log || scale.bands < 2 || !(scale.lo > 0.0) {
        return None;
    }
    let whole = |x: f64| (x - x.round()).abs() < 1e-6;
    let (a, b) = (scale.lo.log10(), scale.hi.log10());
    Some(whole(a) && whole((b - a) / scale.bands as f64))
}

/// 1, 2 or 5 times a power of ten, the nearest at or above `x`.
pub fn nice_step(x: f64) -> f64 {
    if !(x > 0.0) || !x.is_finite() {
        return 1.0;
    }
    let p = 10f64.powf(x.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * p)
        .find(|&s| s >= x * 0.999)
        .unwrap_or(10.0 * p)
}

/// The labels of `values`, in one notation ([`Numbers`]).
pub fn labels(values: &[f64], style: &TickStyle) -> Vec<String> {
    let max = style.digits.map_or(4, |d| d.clamp(1, 6)) as i32;
    let exponent = match style.numbers {
        Numbers::Decimal => false,
        Numbers::Powers => true,
        Numbers::Auto => values
            .iter()
            .any(|&v| v != 0.0 && v.is_finite() && !(1e-2..1e4).contains(&v.abs())),
    };
    values
        .iter()
        .map(|&v| {
            if v == 0.0 || !v.is_finite() {
                return if v == 0.0 { "0".into() } else { format!("{v}") };
            }
            let d = digits_for(v, max);
            if exponent {
                let (m, e) = mantissa(v, d);
                let m = trimmed(format!("{m:.*}", (d - 1).max(0) as usize));
                match (style.numbers, m.as_str()) {
                    (Numbers::Powers, "1") => format!("10{}", superscript(e)),
                    (Numbers::Powers, "-1") => format!("-10{}", superscript(e)),
                    (Numbers::Powers, _) => format!("{m}·10{}", superscript(e)),
                    _ => format!("{m}e{e}"),
                }
            } else {
                let e = v.abs().log10().floor() as i32;
                trimmed(format!("{v:.*}", (d - 1 - e).max(0) as usize))
            }
        })
        .collect()
}

/// The fewest significant digits (up to `max`) that write `v` to 0.05 %.
fn digits_for(v: f64, max: i32) -> i32 {
    (1..=max)
        .find(|&d| {
            let (m, e) = mantissa(v, d);
            ((m * 10f64.powi(e) - v) / v).abs() <= 5e-4
        })
        .unwrap_or(max)
}

/// `v` as a mantissa from 1 to 10 rounded to `d` significant digits, and
/// its power of ten.
fn mantissa(v: f64, d: i32) -> (f64, i32) {
    let mut e = v.abs().log10().floor() as i32;
    let q = 10f64.powi(d - 1);
    let mut m = (v / 10f64.powi(e) * q).round() / q;
    if m.abs() >= 10.0 {
        m /= 10.0;
        e += 1;
    }
    (m, e)
}

fn trimmed(s: String) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// An exponent in superscript digits.
pub fn superscript(e: i32) -> String {
    const D: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    let mut s = String::new();
    if e < 0 {
        s.push('⁻');
    }
    for c in e.unsigned_abs().to_string().chars() {
        s.push(D[c.to_digit(10).unwrap_or(0) as usize]);
    }
    s
}

/// A scale with its ends moved out to round numbers: decades on a log
/// scale, multiples of a 1-2-5 step (a tenth of the range or so) on a
/// linear one.
pub fn rounded(scale: &Scale) -> Scale {
    let mut s = *scale;
    if s.log {
        if s.lo > 0.0 && s.hi > s.lo {
            s.lo = 10f64.powf(s.lo.log10().floor());
            s.hi = 10f64.powf(s.hi.log10().ceil());
            if !(s.hi > s.lo) {
                s.hi = 10.0 * s.lo;
            }
        }
    } else if s.hi > s.lo {
        let step = nice_step((s.hi - s.lo) / 10.0);
        s.lo = (s.lo / step).floor() * step;
        s.hi = (s.hi / step).ceil() * step;
    }
    s
}

/// How a character is drawn: the glyph, its size relative to the text's and
/// how far its baseline is raised (in text sizes). The font has no
/// superscript minus: a small minus sign, raised, stands for it.
fn glyph_of(c: char) -> (char, f32, f32) {
    match c {
        '⁻' => ('−', 0.6, 0.38),
        c => (c, 1.0, 0.0),
    }
}

/// The width of `text` in IBM Plex Mono at `size` (points or pixels).
pub fn text_width(text: &str, size: f32) -> f32 {
    let f = font();
    let mut w = 0.0;
    let mut last = None;
    for c in text.chars() {
        let (c, k, _) = glyph_of(c);
        let sf = f.as_scaled(PxScale::from(size * k));
        let g = sf.glyph_id(c);
        if let Some(p) = last.filter(|_| k == 1.0) {
            w += sf.kern(p, g);
        }
        w += sf.h_advance(g);
        last = Some(g);
    }
    w
}

/// A rectangle, `x0 < x1` and `y0 < y1` (y down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { x0, y0, x1, y1 }
    }
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }
}

/// Where a legend goes in its view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    #[default]
    BottomLeft,
    BottomRight,
}

/// What a legend shows and how.
#[derive(Clone, Debug)]
pub struct Spec<'a> {
    pub scale: &'a Scale,
    pub title: &'a str,
    pub style: TickStyle,
    pub vertical: bool,
    /// The bar's length and thickness (points); the length grows if its
    /// end labels need more.
    pub length: f32,
    pub thickness: f32,
    /// The numbers' size (points); the title is a little larger.
    pub text: f32,
    pub corner: Corner,
    /// The colours below and above the scale: a triangle at each end.
    pub below: Option<Rgb>,
    pub above: Option<Rgb>,
}

/// A text, anchored at `(x, y)`: `ax` 0 its left, 0.5 its middle, 1 its
/// right; `ay` likewise from its top.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub x: f32,
    pub y: f32,
    pub ax: f32,
    pub ay: f32,
    pub size: f32,
    pub text: String,
}

/// A legend laid out: everything in points, in the area given.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// The box behind it (opaque).
    pub back: Rect,
    pub bar: Rect,
    /// The bar's colours, low end first.
    pub pieces: Vec<(Rect, Rgb)>,
    /// Triangles beyond the bar's ends for the colours outside the scale.
    pub arrows: Vec<([[f32; 2]; 3], Rgb)>,
    /// Tick marks, as segments `[x0, y0, x1, y1]`.
    pub marks: Vec<[f32; 4]>,
    pub texts: Vec<Text>,
}

/// Margin between the box and the area's edges, and inside the box.
const MARGIN: f32 = 14.0;

/// Lay `spec` out in `area`: in its corner, the box sized to the measured
/// text; tick labels that would overlap (the ends always kept) left out
/// with their marks.
pub fn layout(spec: &Spec, area: Rect) -> Layout {
    let s = spec.scale;
    let size = spec.text.max(6.0);
    let title_size = size * 1.1;
    let line = size * 1.25;
    let pad = (0.7 * size).max(6.0);
    let gap = 0.6 * size;
    let values = tick_values(s, spec.style.ticks);
    let words = labels(&values, &spec.style);
    let widths: Vec<f32> = words.iter().map(|w| text_width(w, size)).collect();
    let title_w = text_width(spec.title, title_size);
    let thick = spec.thickness.max(4.0);
    let arrow = |on: bool| if on { thick } else { 0.0 };
    let (a0, a1) = (arrow(spec.below.is_some()), arrow(spec.above.is_some()));
    let n = values.len();
    let mut out = Layout::default();

    // The box's size, then its place; the parts from its corner.
    // The bar no longer than the area holds (the rest of the box beside it).
    let (room_w, room_h) = (area.width() - 2.0 * MARGIN, area.height() - 2.0 * MARGIN);
    // Exact ticks are all kept: the bar made long enough for their labels
    // if the area allows, else (horizontal) the labels on two rows.
    let exact = spec.style.ticks.exact(s) && n > 2;
    let anchor = |k: usize| {
        if k == 0 {
            0.0
        } else if k + 1 == n {
            1.0
        } else {
            0.5
        }
    };
    // The spacing that keeps labels `d` apart clear of each other.
    let spacing = |d: usize| {
        (0..n.saturating_sub(d))
            .map(|k| {
                let room = (1.0 - anchor(k)) * widths[k] + anchor(k + d) * widths[k + d] + gap;
                room / d as f32
            })
            .fold(0.0, f32::max)
    };
    let mut rows = 1;
    let (w, h, length);
    if spec.vertical {
        // Half a line above and below the bar for its end labels.
        let rest = 2.0 * pad + line + 4.0 + 0.5 * line + a1 + a0 + 0.5 * line;
        let mut l = spec.length.min(room_h - rest).max(2.0 * line);
        if exact {
            l = l.max(((n - 1) as f32 * (line + 1.0)).min(room_h - rest));
        }
        length = l;
        let labels_w = widths.iter().cloned().fold(0.0, f32::max);
        let bar_w = thick + 4.0 + 4.0 + labels_w;
        w = 2.0 * pad + title_w.max(bar_w);
        h = rest + length;
    } else {
        // Room for both end labels side by side.
        let ends = widths.first().copied().unwrap_or(0.0) + widths.last().copied().unwrap_or(0.0);
        let rest = 2.0 * pad + a0 + a1;
        let room = room_w - rest;
        let mut l = spec.length.min(room).max(ends + 2.0 * gap);
        if exact {
            let (one, two) = ((n - 1) as f32 * spacing(1), (n - 1) as f32 * spacing(2));
            if l < one {
                if one <= room {
                    l = one;
                } else {
                    rows = 2;
                    l = l.max(two.min(room));
                }
            }
        }
        length = l;
        w = 2.0 * pad + (a0 + length + a1).max(title_w);
        h = 2.0 * pad + line + 4.0 + thick + 4.0 + rows as f32 * line;
    }
    let (left, top) = match spec.corner {
        Corner::TopLeft => (true, true),
        Corner::TopRight => (false, true),
        Corner::BottomLeft => (true, false),
        Corner::BottomRight => (false, false),
    };
    let x0 = if left {
        area.x0 + MARGIN
    } else {
        area.x1 - MARGIN - w
    };
    let y0 = if top {
        area.y0 + MARGIN
    } else {
        area.y1 - MARGIN - h
    };
    out.back = Rect::new(x0, y0, x0 + w, y0 + h);
    out.texts.push(Text {
        x: x0 + pad,
        y: y0 + pad,
        ax: 0.0,
        ay: 0.0,
        size: title_size,
        text: spec.title.to_string(),
    });
    let top_of_bar = y0 + pad + line + 4.0;
    let pieces = 128;
    if spec.vertical {
        let bx = x0 + pad;
        let top_of_bar = top_of_bar + 0.5 * line;
        let bar = Rect::new(bx, top_of_bar + a1, bx + thick, top_of_bar + a1 + length);
        out.bar = bar;
        for k in 0..pieces {
            let t0 = k as f32 / pieces as f32;
            let t1 = (k + 1) as f32 / pieces as f32;
            let c = s.colour(s.value((k as f64 + 0.5) / pieces as f64));
            let r = Rect::new(
                bar.x0,
                bar.y1 - bar.height() * t1,
                bar.x1,
                bar.y1 - bar.height() * t0,
            );
            out.pieces.push((r, c));
        }
        let mid = 0.5 * (bar.x0 + bar.x1);
        if let Some(c) = spec.below {
            out.arrows
                .push(([[bar.x0, bar.y1], [bar.x1, bar.y1], [mid, bar.y1 + a0]], c));
        }
        if let Some(c) = spec.above {
            out.arrows
                .push(([[bar.x0, bar.y0], [bar.x1, bar.y0], [mid, bar.y0 - a1]], c));
        }
        let y = |v: f64| bar.y1 - bar.height() * s.fraction(v) as f32;
        let keep = if exact {
            (0..n).collect()
        } else {
            kept(
                n,
                |k| {
                    let c = y(values[k]);
                    (c - 0.5 * line, c + 0.5 * line)
                },
                1.0,
            )
        };
        for k in keep {
            let c = y(values[k]);
            out.marks.push([bar.x1, c, bar.x1 + 4.0, c]);
            out.texts.push(Text {
                x: bar.x1 + 8.0,
                y: c,
                ax: 0.0,
                ay: 0.5,
                size,
                text: words[k].clone(),
            });
        }
    } else {
        let bx = x0 + pad + a0;
        let bar = Rect::new(bx, top_of_bar, bx + length, top_of_bar + thick);
        out.bar = bar;
        for k in 0..pieces {
            let t0 = k as f32 / pieces as f32;
            let t1 = (k + 1) as f32 / pieces as f32;
            let c = s.colour(s.value((k as f64 + 0.5) / pieces as f64));
            let r = Rect::new(
                bar.x0 + bar.width() * t0,
                bar.y0,
                bar.x0 + bar.width() * t1,
                bar.y1,
            );
            out.pieces.push((r, c));
        }
        let mid = 0.5 * (bar.y0 + bar.y1);
        if let Some(c) = spec.below {
            out.arrows
                .push(([[bar.x0, bar.y0], [bar.x0, bar.y1], [bar.x0 - a0, mid]], c));
        }
        if let Some(c) = spec.above {
            out.arrows
                .push(([[bar.x1, bar.y0], [bar.x1, bar.y1], [bar.x1 + a1, mid]], c));
        }
        let x = |v: f64| bar.x0 + bar.width() * s.fraction(v) as f32;
        // The ends' labels inside the bar's span; the others centred.
        let keep = if exact {
            (0..n).collect()
        } else {
            kept(
                n,
                |k| {
                    let c = x(values[k]);
                    let a = anchor(k);
                    (c - a * widths[k], c + (1.0 - a) * widths[k])
                },
                gap,
            )
        };
        for k in keep {
            let c = x(values[k]);
            // On two rows, every other label on the second, its mark
            // reaching down to it.
            let row = if rows == 2 { k % 2 } else { 0 } as f32;
            out.marks.push([c, bar.y1, c, bar.y1 + 4.0 + row * line]);
            out.texts.push(Text {
                x: c,
                y: bar.y1 + 5.0 + row * line,
                ax: anchor(k),
                ay: 0.0,
                size,
                text: words[k].clone(),
            });
        }
    }
    out
}

/// A legend as a strip to put under an image `width` pixels wide (a point
/// a pixel): on white, horizontal, its bar across the width; `spec`'s
/// orientation, corner and length are not used, and its text is scaled
/// with the width when it is 0. The strip's RGB rows and their number.
pub fn strip(spec: &Spec, width: usize) -> (Vec<u8>, usize) {
    let text = if spec.text > 0.0 {
        spec.text
    } else {
        (width as f32 / 90.0).clamp(11.0, 24.0)
    };
    let spec = Spec {
        vertical: false,
        corner: Corner::TopLeft,
        length: f32::MAX,
        text,
        ..spec.clone()
    };
    // Laid out once to learn its height, in an area as tall as needed.
    let tall = layout(&spec, Rect::new(0.0, 0.0, width as f32, 1e5));
    let height = (tall.back.y1 + 0.5 * MARGIN).ceil() as usize;
    let mut rgb = vec![255u8; 3 * width * height];
    let white = Colours {
        border: [255; 3],
        ..Colours::LIGHT
    };
    rasterize(
        &mut Canvas {
            rgb: &mut rgb,
            width,
            height,
        },
        &tall,
        1.0,
        white,
    );
    (rgb, height)
}

/// Which of `n` labels to draw, given each one's extent along the bar: the
/// two ends, then those between that keep `gap` from every one kept (in
/// order).
fn kept(n: usize, extent: impl Fn(usize) -> (f32, f32), gap: f32) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let mut keep = vec![0];
    if n > 1 {
        keep.push(n - 1);
    }
    let clear = |a: (f32, f32), b: (f32, f32)| a.1 + gap <= b.0 || b.1 + gap <= a.0;
    for k in 1..n.saturating_sub(1) {
        if keep.iter().all(|&j| clear(extent(j), extent(k))) {
            keep.push(k);
        }
    }
    keep.sort_unstable();
    keep
}

/// The colours a legend is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colours {
    pub back: Rgb,
    pub border: Rgb,
    pub ink: Rgb,
}

impl Colours {
    /// Dark text on white, as on paper.
    pub const LIGHT: Colours = Colours {
        back: [255, 255, 255],
        border: [170, 170, 170],
        ink: [30, 30, 30],
    };
}

/// An RGB image to draw into: `width × height` pixels, rows from the top.
pub struct Canvas<'a> {
    pub rgb: &'a mut [u8],
    pub width: usize,
    pub height: usize,
}

impl Canvas<'_> {
    fn blend(&mut self, x: i64, y: i64, c: Rgb, a: f32) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let k = 3 * (y as usize * self.width + x as usize);
        let a = a.clamp(0.0, 1.0);
        for i in 0..3 {
            let old = self.rgb[k + i] as f32;
            self.rgb[k + i] = (old + a * (c[i] as f32 - old)).round() as u8;
        }
    }

    fn fill(&mut self, r: Rect, c: Rgb) {
        let (x0, x1) = (r.x0.round() as i64, r.x1.round() as i64);
        let (y0, y1) = (r.y0.round() as i64, r.y1.round() as i64);
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend(x, y, c, 1.0);
            }
        }
    }

    fn triangle(&mut self, p: [[f32; 2]; 3], c: Rgb) {
        let xs = p.map(|q| q[0]);
        let ys = p.map(|q| q[1]);
        let (x0, x1) = (
            xs.iter().cloned().fold(f32::MAX, f32::min),
            xs.iter().cloned().fold(f32::MIN, f32::max),
        );
        let (y0, y1) = (
            ys.iter().cloned().fold(f32::MAX, f32::min),
            ys.iter().cloned().fold(f32::MIN, f32::max),
        );
        let side = |a: [f32; 2], b: [f32; 2], q: [f32; 2]| {
            (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0])
        };
        for y in y0.floor() as i64..=y1.ceil() as i64 {
            for x in x0.floor() as i64..=x1.ceil() as i64 {
                let q = [x as f32 + 0.5, y as f32 + 0.5];
                let s = [
                    side(p[0], p[1], q),
                    side(p[1], p[2], q),
                    side(p[2], p[0], q),
                ];
                if s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0) {
                    self.blend(x, y, c, 1.0);
                }
            }
        }
    }

    fn text(&mut self, t: &Text, scale: f32, c: Rgb) {
        let f = font();
        let px = t.size * scale;
        let sf = f.as_scaled(PxScale::from(px));
        let w = text_width(&t.text, px);
        let h = sf.ascent() - sf.descent();
        let left = t.x * scale - t.ax * w;
        let top = t.y * scale - t.ay * h;
        let mut x = left;
        let mut last = None;
        for ch in t.text.chars() {
            let (ch, k, rise) = glyph_of(ch);
            let sf = f.as_scaled(PxScale::from(px * k));
            let id = sf.glyph_id(ch);
            if let Some(p) = last.filter(|_| k == 1.0) {
                x += sf.kern(p, id);
            }
            let baseline = top + f.as_scaled(PxScale::from(px)).ascent() - rise * px;
            let g = id.with_scale_and_position(px * k, ab_glyph::point(x, baseline));
            if let Some(o) = f.outline_glyph(g) {
                let b = o.px_bounds();
                o.draw(|gx, gy, a| {
                    self.blend(b.min.x as i64 + gx as i64, b.min.y as i64 + gy as i64, c, a);
                });
            }
            x += sf.h_advance(id);
            last = Some(id);
        }
    }
}

/// Draw `layout` into `canvas`, `scale` pixels to the point.
pub fn rasterize(canvas: &mut Canvas, layout: &Layout, scale: f32, colours: Colours) {
    let s = |r: Rect| Rect::new(r.x0 * scale, r.y0 * scale, r.x1 * scale, r.y1 * scale);
    let b = s(layout.back);
    let line = scale.max(1.0);
    canvas.fill(b, colours.border);
    canvas.fill(
        Rect::new(b.x0 + line, b.y0 + line, b.x1 - line, b.y1 - line),
        colours.back,
    );
    for &(r, c) in &layout.pieces {
        // Each piece to the next one's start: no seams.
        let r = s(r);
        canvas.fill(
            Rect::new(r.x0.floor(), r.y0.floor(), r.x1.ceil(), r.y1.ceil()),
            c,
        );
    }
    for &(p, c) in &layout.arrows {
        // Edged, so that a colour like the box's still shows.
        let p = p.map(|q| [q[0] * scale, q[1] * scale]);
        let mid = [0, 1].map(|i| (p[0][i] + p[1][i] + p[2][i]) / 3.0);
        let inset = |q: [f32; 2]| {
            let d = [q[0] - mid[0], q[1] - mid[1]];
            let n = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-3);
            let k = ((n - 1.6 * line) / n).max(0.0);
            [mid[0] + k * d[0], mid[1] + k * d[1]]
        };
        canvas.triangle(p, colours.ink);
        canvas.triangle(p.map(inset), c);
    }
    for m in &layout.marks {
        let r = if m[0] == m[2] {
            Rect::new(
                m[0] * scale - 0.5 * line,
                m[1] * scale,
                m[0] * scale + 0.5 * line,
                m[3] * scale,
            )
        } else {
            Rect::new(
                m[0] * scale,
                m[1] * scale - 0.5 * line,
                m[2] * scale,
                m[1] * scale + 0.5 * line,
            )
        };
        canvas.fill(r, colours.ink);
    }
    for t in &layout.texts {
        canvas.text(t, scale, colours.ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(v: &[f64]) -> Vec<String> {
        labels(v, &TickStyle::default())
    }

    #[test]
    fn one_notation_per_bar() {
        // Some need an exponent: all have one.
        assert_eq!(auto(&[1.0, 1e3, 1e6, 1e12]), ["1e0", "1e3", "1e6", "1e12"]);
        assert_eq!(auto(&[3.2716e-6, 1e-3, 1.0]), ["3.27e-6", "1e-3", "1e0"]);
        assert_eq!(auto(&[3.2856e-6]), ["3.286e-6"]);
        // None does: decimals, each as long as it needs.
        assert_eq!(auto(&[0.5, 2.0, 25.0, 1000.0]), ["0.5", "2", "25", "1000"]);
        assert_eq!(auto(&[0.0, 0.25, 7.3]), ["0", "0.25", "7.3"]);
        let powers = TickStyle {
            numbers: Numbers::Powers,
            ..TickStyle::default()
        };
        assert_eq!(
            labels(&[1.0, 2.5e3, 1e12], &powers),
            ["10⁰", "2.5·10³", "10¹²"]
        );
        let decimal = TickStyle {
            numbers: Numbers::Decimal,
            digits: Some(2),
            ..TickStyle::default()
        };
        assert_eq!(
            labels(&[1e-3, 3.24159, 1e6], &decimal),
            ["0.001", "3.2", "1000000"]
        );
        // A mantissa rounding to 10 moves to the next power.
        let two = TickStyle {
            digits: Some(2),
            ..TickStyle::default()
        };
        assert_eq!(labels(&[9.996e5], &two), ["1e6"]);
    }

    #[test]
    fn tick_counts() {
        let s = Scale::log(1.0, 1e12);
        assert_eq!(tick_values(&s, Ticks::Many).len(), 13);
        // Exact: as many as asked, evenly in decades.
        let every: Vec<f64> = (0..=12).map(|e| 10f64.powi(e)).collect();
        assert_eq!(tick_values(&s, Ticks::Count(13)), every);
        assert_eq!(tick_values(&s, Ticks::Count(5)), [1.0, 1e3, 1e6, 1e9, 1e12]);
        let six = tick_values(&s, Ticks::Count(6));
        assert_eq!(six.len(), 6);
        assert!((six[1] - 10f64.powf(2.4)).abs() < 1e-6 * six[1]);
        let lin = tick_values(&Scale::linear(0.0, 10.0), Ticks::Count(6));
        assert_eq!(lin, [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(
            tick_values(&Scale::linear(0.0, 1.0), Ticks::Count(4))[1],
            0.333333333333
        );
    }

    #[test]
    fn ticks_at_the_band_edges() {
        let mut s = Scale::log(1.0, 1e12);
        // No bands: as Auto.
        assert_eq!(tick_values(&s, Ticks::Edges), tick_values(&s, Ticks::Auto));
        assert!(!Ticks::Edges.exact(&s));
        assert_eq!(edges_on_decades(&s), None);
        s.bands = 4;
        let edges = tick_values(&s, Ticks::Edges);
        assert_eq!(edges, [1.0, 1e3, 1e6, 1e9, 1e12]);
        assert_eq!(edges_on_decades(&s), Some(true));
        // Each inner edge is where the colour changes, and only there.
        for w in edges.windows(2) {
            assert_eq!(s.colour(w[0] * 1.01), s.colour(w[1] * 0.99));
        }
        for &e in &edges[1..edges.len() - 1] {
            assert_ne!(s.colour(e * 0.99), s.colour(e * 1.01));
        }
        s.bands = 5;
        assert_eq!(tick_values(&s, Ticks::Edges).len(), 6);
        assert_eq!(edges_on_decades(&s), Some(false));
    }

    #[test]
    fn rounded_ends() {
        let s = rounded(&Scale::log(3.27e-6, 4.1e3));
        assert_eq!((s.lo, s.hi), (1e-6, 1e4));
        let s = rounded(&Scale::linear(0.13, 9.7));
        assert_eq!((s.lo, s.hi), (0.0, 10.0));
    }

    #[test]
    fn labels_never_overlap_and_stay_in_the_box() {
        let area = Rect::new(0.0, 0.0, 800.0, 600.0);
        for vertical in [false, true] {
            for ticks in [Ticks::Auto, Ticks::Many, Ticks::Count(12)] {
                let scale = Scale::log(3.2716e-6, 4.1e12);
                let spec = Spec {
                    scale: &scale,
                    title: "flux [1/cm²]",
                    style: TickStyle {
                        ticks,
                        ..TickStyle::default()
                    },
                    vertical,
                    length: 60.0,
                    thickness: 10.0,
                    text: 12.0,
                    corner: Corner::BottomRight,
                    below: Some([0, 0, 255]),
                    above: None,
                };
                let l = layout(&spec, area);
                let boxes: Vec<Rect> = l
                    .texts
                    .iter()
                    .map(|t| {
                        let w = text_width(&t.text, t.size);
                        let h = t.size * 1.2;
                        Rect::new(
                            t.x - t.ax * w,
                            t.y - t.ay * h,
                            t.x + (1.0 - t.ax) * w,
                            t.y + (1.0 - t.ay) * h,
                        )
                    })
                    .collect();
                for (i, a) in boxes.iter().enumerate() {
                    let inside = a.x0 >= l.back.x0 - 0.5
                        && a.x1 <= l.back.x1 + 0.5
                        && a.y0 >= l.back.y0 - 0.5
                        && a.y1 <= l.back.y1 + 0.5;
                    assert!(
                        inside,
                        "{vertical} {ticks:?}: {:?} out of {:?}",
                        l.texts[i], l.back
                    );
                    for b in &boxes[i + 1..] {
                        let apart = a.x1 <= b.x0 || b.x1 <= a.x0 || a.y1 <= b.y0 || b.y1 <= a.y0;
                        assert!(apart, "{vertical} {ticks:?}: {a:?} {b:?}");
                    }
                }
                // The ends are always written; the box is in the area.
                assert!(l.texts.iter().any(|t| t.text == "3.27e-6"));
                assert!(l.back.x0 >= 0.0 && l.back.x1 <= 800.0 && l.back.y1 <= 600.0);
                // A bar longer than the area holds is shortened to fit.
                let long = Spec {
                    length: 5000.0,
                    corner: Corner::TopLeft,
                    ..spec.clone()
                };
                let l = layout(&long, area);
                assert!(
                    l.back.y1 <= 600.0 - MARGIN + 0.5 && l.back.x1 <= 800.0 - MARGIN + 0.5,
                    "{:?}",
                    l.back
                );
            }
        }
    }

    #[test]
    fn exact_ticks_are_all_kept() {
        let scale = Scale::log(1e-6, 1e12);
        let spec = |n: u8, vertical: bool, width: f32| {
            let l = layout(
                &Spec {
                    scale: &scale,
                    title: "flux",
                    style: TickStyle {
                        ticks: Ticks::Count(n),
                        ..TickStyle::default()
                    },
                    vertical,
                    length: 100.0,
                    thickness: 10.0,
                    text: 12.0,
                    corner: Corner::BottomLeft,
                    below: None,
                    above: None,
                },
                Rect::new(0.0, 0.0, width, 900.0),
            );
            let ticks: Vec<Text> = l.texts.iter().skip(1).cloned().collect();
            (l, ticks)
        };
        // Room enough: the bar grows, one row.
        let (l, ticks) = spec(7, false, 1000.0);
        assert_eq!(ticks.len(), 7);
        assert!(l.bar.width() > 100.0);
        assert!(ticks.iter().all(|t| t.y == ticks[0].y));
        // Not enough: every other label on a second row, none dropped.
        let (l, ticks) = spec(19, false, 600.0);
        assert_eq!(ticks.len(), 19);
        assert_eq!(l.marks.len(), 19);
        let rows: std::collections::BTreeSet<i32> = ticks.iter().map(|t| t.y as i32).collect();
        assert_eq!(rows.len(), 2);
        assert!(l.back.y1 >= ticks.iter().map(|t| t.y + t.size).fold(0.0, f32::max));
        // Upright: the bar grows to a line per label.
        let (l, ticks) = spec(19, true, 600.0);
        assert_eq!(ticks.len(), 19);
        assert!(l.bar.height() >= 18.0 * 12.0 * 1.25);
    }

    #[test]
    fn a_strip_under_an_image() {
        let mut scale = Scale::log(1.0, 1e12);
        scale.bands = 4;
        let spec = Spec {
            scale: &scale,
            title: "flux",
            style: TickStyle {
                ticks: Ticks::Edges,
                ..TickStyle::default()
            },
            vertical: true,
            length: 10.0,
            thickness: 12.0,
            text: 0.0,
            corner: Corner::BottomRight,
            below: None,
            above: None,
        };
        let (rgb, h) = strip(&spec, 600);
        assert_eq!(rgb.len(), 3 * 600 * h);
        assert!(h > 40 && h < 150, "{h}");
        // White, with the bar's colours across most of the width.
        assert_eq!(&rgb[..3], &[255; 3]);
        // Columns holding a colour of the ramp (not grey): the bar.
        let coloured = (0..600)
            .filter(|&x| {
                (0..h).any(|y| {
                    let p = &rgb[3 * (y * 600 + x)..][..3];
                    p.iter().max().unwrap() - p.iter().min().unwrap() > 40
                })
            })
            .count();
        assert!(coloured > 450, "{coloured}");
    }

    #[test]
    fn a_legend_drawn_into_an_image() {
        let scale = Scale::linear(0.0, 1.0);
        let spec = Spec {
            scale: &scale,
            title: "t",
            style: TickStyle::default(),
            vertical: false,
            length: 100.0,
            thickness: 10.0,
            text: 11.0,
            corner: Corner::TopLeft,
            below: None,
            above: None,
        };
        let l = layout(&spec, Rect::new(0.0, 0.0, 200.0, 150.0));
        let (w, h) = (400, 300);
        let mut rgb = vec![0u8; 3 * w * h];
        rasterize(
            &mut Canvas {
                rgb: &mut rgb,
                width: w,
                height: h,
            },
            &l,
            2.0,
            Colours::LIGHT,
        );
        let px = |x: f32, y: f32| {
            let k = 3 * ((2.0 * y) as usize * w + (2.0 * x) as usize);
            [rgb[k], rgb[k + 1], rgb[k + 2]]
        };
        // The box is opaque white; the bar holds the ramp's colours.
        assert_eq!(px(l.back.x0 + 2.0, l.back.y1 - 2.0), [255; 3]);
        let mid = 0.5 * (l.bar.y0 + l.bar.y1);
        assert_eq!(
            px(l.bar.x0 + 0.2, mid),
            scale.colour(scale.value(0.5 / 128.0))
        );
        // Some ink: the labels were written.
        let dark = rgb
            .chunks(3)
            .filter(|p| p[0] < 100 && p[1] < 100 && p[2] < 100)
            .count();
        assert!(dark > 50, "{dark}");
        // A superscript minus is drawn (from a small minus sign).
        assert!(text_width("10⁻⁶", 12.0) > text_width("10⁶", 12.0));
    }
}
