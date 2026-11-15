//! Colour scales: numbers to colours along a ramp, linear or logarithmic.
//! Density colours ([`Scale::DENSITY`]) and fields over the geometry use
//! them, and colour bars (`crate::legend`) mark them with [`Scale::ticks`]; the GPU gets a
//! scale as a lookup table ([`Scale::lut`]).

use crate::colours::Rgb;
/// Short labels: `1`, `1e-2`, `25`, `0.35`, `3.2e6`.
pub use llevant_scene::field::tick_label;

/// The colours a scale runs through, from its low end to its high end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Ramp {
    /// Dark blue through teal and green to yellow, close to viridis:
    /// perceptually even, readable in grey and by most colour-blind eyes.
    #[default]
    Viridis,
    /// Google's Turbo: dark blue through cyan, green, yellow and red, a
    /// smooth rainbow with the contrast of jet and none of its bands.
    Turbo,
    /// Black through purple and orange to pale yellow (matplotlib's
    /// inferno): perceptually even, prints well.
    Inferno,
    /// Black to white.
    Grey,
}

impl Ramp {
    pub const ALL: [Ramp; 4] = [Ramp::Viridis, Ramp::Turbo, Ramp::Inferno, Ramp::Grey];

    /// Evenly spaced stops, linearly interpolated between.
    fn stops(self) -> &'static [[f64; 3]] {
        match self {
            Ramp::Viridis => &[
                [68.0, 1.0, 84.0],
                [65.0, 68.0, 135.0],
                [42.0, 120.0, 142.0],
                [34.0, 168.0, 132.0],
                [122.0, 209.0, 81.0],
                [253.0, 231.0, 37.0],
            ],
            Ramp::Inferno => &[
                [0.0, 0.0, 4.0],
                [31.0, 12.0, 72.0],
                [85.0, 15.0, 109.0],
                [136.0, 34.0, 106.0],
                [186.0, 54.0, 85.0],
                [227.0, 89.0, 51.0],
                [249.0, 140.0, 10.0],
                [249.0, 201.0, 50.0],
                [252.0, 255.0, 164.0],
            ],
            Ramp::Grey => &[[0.0, 0.0, 0.0], [255.0, 255.0, 255.0]],
            Ramp::Turbo => &[],
        }
    }

    /// The colour at `t` in `[0, 1]` (clamped).
    pub fn at(self, t: f64) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        if self == Ramp::Turbo {
            return turbo(t);
        }
        let stops = self.stops();
        let x = t * (stops.len() - 1) as f64;
        let k = (x.floor() as usize).min(stops.len() - 2);
        let f = x - k as f64;
        [0, 1, 2].map(|i| (stops[k][i] + f * (stops[k + 1][i] - stops[k][i])).round() as u8)
    }

    pub fn name(self) -> &'static str {
        match self {
            Ramp::Viridis => "viridis",
            Ramp::Turbo => "turbo",
            Ramp::Inferno => "inferno",
            Ramp::Grey => "grey",
        }
    }

    /// The ramp called `name` (case aside; `gray` too).
    pub fn from_name(name: &str) -> Option<Ramp> {
        let n = name.trim().to_ascii_lowercase();
        let n = if n == "gray" { "grey".to_string() } else { n };
        Ramp::ALL.into_iter().find(|r| r.name() == n)
    }
}

/// Turbo at `t`, from its published polynomial fit (Mikhailov, 2019).
fn turbo(t: f64) -> Rgb {
    let r = 0.13572138
        + t * (4.61539260
            + t * (-42.66032258 + t * (132.13108234 + t * (-152.94239396 + t * 59.28637943))));
    let g = 0.09140261
        + t * (2.19418839
            + t * (4.84296658 + t * (-14.18503333 + t * (4.27729857 + t * 2.82956604))));
    let b = 0.10667330
        + t * (12.64194608
            + t * (-60.58204836 + t * (110.36276771 + t * (-89.90310912 + t * 27.34824973))));
    [r, g, b].map(|c| (255.0 * c.clamp(0.0, 1.0)).round() as u8)
}

/// Values from `lo` to `hi` along a ramp; outside them, its ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub lo: f64,
    pub hi: f64,
    /// Logarithmic (`lo` must then be positive; values at or below zero
    /// take the low end).
    pub log: bool,
    pub ramp: Ramp,
    /// Discrete colours: this many bands of one colour each (on a log
    /// scale, of equal width in decades); 0 for a continuous ramp.
    pub bands: u32,
}

impl Scale {
    /// Densities in g/cm³, from gases to tungsten and beyond, on a log
    /// scale ([`ColourBy::Density`](crate::colours::ColourBy::Density)).
    pub const DENSITY: Scale = Scale {
        lo: 1e-4,
        hi: 25.0,
        log: true,
        ramp: Ramp::Viridis,
        bands: 0,
    };

    pub fn linear(lo: f64, hi: f64) -> Scale {
        Scale {
            lo,
            hi,
            log: false,
            ramp: Ramp::default(),
            bands: 0,
        }
    }

    pub fn log(lo: f64, hi: f64) -> Scale {
        Scale {
            lo,
            hi,
            log: true,
            ramp: Ramp::default(),
            bands: 0,
        }
    }

    /// From the finite values (positive ones only for `log`) of a field:
    /// their 1st to 99th percentiles, so that a few outliers do not squeeze
    /// the rest into one colour. `None` without such values.
    pub fn auto(values: impl IntoIterator<Item = f64>, log: bool) -> Option<Scale> {
        let mut v: Vec<f64> = values
            .into_iter()
            .filter(|x| x.is_finite() && (!log || *x > 0.0))
            .collect();
        if v.is_empty() {
            return None;
        }
        v.sort_unstable_by(f64::total_cmp);
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        let (mut lo, mut hi) = (at(0.01), at(0.99));
        if !(lo < hi) {
            (lo, hi) = (v[0], v[v.len() - 1]);
        }
        if !(lo < hi) {
            // One value: a scale around it.
            (lo, hi) = if log {
                (lo / 10.0, lo * 10.0)
            } else if lo == 0.0 {
                (-1.0, 1.0)
            } else {
                (lo - 0.5 * lo.abs(), lo + 0.5 * lo.abs())
            };
        }
        Some(Scale {
            lo,
            hi,
            log,
            ramp: Ramp::default(),
            bands: 0,
        })
    }

    /// Where `v` lies on the scale: 0 at `lo`, 1 at `hi`, clamped (NaN
    /// gives 0).
    pub fn fraction(&self, v: f64) -> f64 {
        let t = if self.log {
            (v.max(1e-30).log10() - self.lo.log10()) / (self.hi.log10() - self.lo.log10())
        } else {
            (v - self.lo) / (self.hi - self.lo)
        };
        if t.is_nan() {
            0.0
        } else {
            t.clamp(0.0, 1.0)
        }
    }

    /// The value at fraction `t` of the scale ([`Scale::fraction`]'s
    /// inverse).
    pub fn value(&self, t: f64) -> f64 {
        if self.log {
            10f64.powf(self.lo.log10() + t * (self.hi.log10() - self.lo.log10()))
        } else {
            self.lo + t * (self.hi - self.lo)
        }
    }

    pub fn colour(&self, v: f64) -> Rgb {
        self.ramp.at(self.banded(self.fraction(v)))
    }

    /// Fraction `t` of the scale where its colour is taken: `t` itself on
    /// a continuous ramp, the middle of its band with [`Scale::bands`] (the
    /// first band at the ramp's low end, the last at its high end).
    pub fn banded(&self, t: f64) -> f64 {
        if self.bands < 2 {
            return t;
        }
        let n = self.bands as f64;
        let k = (t * n).floor().clamp(0.0, n - 1.0);
        k / (n - 1.0)
    }

    /// `n` colours evenly spaced along the scale's fraction (for a GPU,
    /// which maps the fraction itself), bands included.
    pub fn lut(&self, n: usize) -> Vec<Rgb> {
        (0..n)
            .map(|k| {
                let t = k as f64 / (n.max(2) - 1) as f64;
                self.ramp.at(self.banded(t))
            })
            .collect()
    }

    /// Values to mark on a colour bar, with their labels: both ends and, in
    /// between, powers of ten (log) or round numbers (linear), about six in
    /// all, in one notation ([`crate::legend`]'s defaults).
    pub fn ticks(&self) -> Vec<(f64, String)> {
        let style = crate::legend::TickStyle::default();
        let values = crate::legend::tick_values(self, style.ticks);
        let labels = crate::legend::labels(&values, &style);
        values.into_iter().zip(labels).collect()
    }
}

/// A value to four significant digits: `300`, `0.01235`, `1.235e-5`.
pub fn value_label(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    let e = v.abs().log10().floor() as i32;
    if (-3..5).contains(&e) {
        let s = format!("{:.*}", (3 - e).max(0) as usize, v);
        if s.contains('.') {
            return s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        return s;
    }
    format!("{v:.3e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_by_name_and_their_ends() {
        for r in Ramp::ALL {
            assert_eq!(Ramp::from_name(r.name()), Some(r));
        }
        assert_eq!(Ramp::from_name("Gray"), Some(Ramp::Grey));
        assert_eq!(Ramp::Grey.at(0.0), [0, 0, 0]);
        assert_eq!(Ramp::Grey.at(1.0), [255, 255, 255]);
        // Turbo: blue near its low end, red near its high end (its very
        // ends are near black).
        let [r0, _, b0] = Ramp::Turbo.at(0.1);
        let [r1, _, b1] = Ramp::Turbo.at(0.9);
        assert!(b0 > r0 && r1 > b1, "{r0} {b0} {r1} {b1}");
    }

    #[test]
    fn bands_take_one_colour_each() {
        let s = Scale {
            bands: 4,
            ..Scale::linear(0.0, 4.0)
        };
        assert_eq!(s.colour(0.1), s.colour(0.9));
        assert_ne!(s.colour(0.9), s.colour(1.1));
        assert_eq!(s.colour(0.0), Ramp::Viridis.at(0.0));
        assert_eq!(s.colour(4.0), Ramp::Viridis.at(1.0));
        let lut = s.lut(256);
        let distinct: std::collections::HashSet<_> = lut.iter().collect();
        assert_eq!(distinct.len(), 4);
    }

    #[test]
    fn density_ticks_and_ends() {
        let s = Scale::DENSITY;
        let labels: Vec<String> = s.ticks().into_iter().map(|(_, l)| l).collect();
        // One notation for the whole bar.
        assert_eq!(labels, ["1e-4", "1e-2", "1e0", "2.5e1"]);
        assert_eq!(s.colour(1e-4), [68, 1, 84]);
        assert_eq!(s.colour(1e9), [253, 231, 37]);
        assert_eq!(s.colour(f64::NAN), [68, 1, 84]);
    }

    #[test]
    fn linear_ticks_are_round() {
        let labels: Vec<String> = Scale::linear(0.0, 7.3)
            .ticks()
            .into_iter()
            .map(|(_, l)| l)
            .collect();
        assert_eq!(labels, ["0", "2", "4", "6", "7.3"]);
        assert!((Scale::log(1.0, 100.0).value(0.5) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn values_have_four_digits() {
        assert_eq!(value_label(300.0), "300");
        assert_eq!(value_label(0.0123456), "0.01235");
        assert_eq!(value_label(1.5), "1.5");
        assert_eq!(value_label(12345.6), "12346");
        assert_eq!(value_label(1.23456e-5), "1.235e-5");
    }

    #[test]
    fn auto_ignores_outliers() {
        let mut v: Vec<f64> = (1..=1000).map(f64::from).collect();
        v.push(1e12);
        let s = Scale::auto(v, false).unwrap();
        assert!(s.hi < 1001.0 && s.lo >= 1.0, "{s:?}");
        assert_eq!(Scale::auto([3.0], true).unwrap().lo, 0.3);
        assert!(Scale::auto([f64::NAN, -1.0], true).is_none());
    }
}
