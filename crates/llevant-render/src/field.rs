//! Fields drawn over slices and renders. The numbers are
//! [`llevant_scene::field::Field`]; how they are drawn is a [`FieldLook`]
//! (a colour [`Scale`] and an opacity), and an [`Overlay`] pairs the two.
//!
//! Under a field, the cells are a neutral grey ([`NO_VALUE`]; on the grey
//! ramp, whose middle is that grey, a slightly blue one,
//! [`NO_VALUE_ON_GREY`]: [`FieldLook::no_value`]) by default
//! ([`Under::Neutral`]): a cell the field says nothing about (or a point
//! outside a mesh) is that grey, so that no colour but the scale's reads as
//! a value, and the field's colour goes over the grey `opacity` of the way
//! ([`mix`]), fading towards it rather than towards a cell's colour.
//! [`Under::Geometry`] keeps the geometry's colours under the field and
//! where it has no value instead. Values below the scale take its low end's
//! colour, above it its high end's (a zero on a log scale is below it): only
//! the lack of a value is grey. Gaps and problems keep their colours. In
//! renders the field's colour and the grey are shaded flat by the angle to
//! the surface whatever the colouring. The viewer's GPU pass
//! (`llevant-viewer/src/view3d/shaders/field.wgsl`) does the same.

use std::sync::Arc;

use llevant_scene::field::Field;

use crate::colours::Rgb;
use crate::scale::Scale;

/// The colour of a cell without a value under a neutral base, and the
/// base itself ([`Under::Neutral`]).
pub const NO_VALUE: Rgb = [140, 140, 140];

/// [`NO_VALUE`] over the grey ramp, which runs through it: a grey with a
/// tint of blue, which no value of that ramp has.
pub const NO_VALUE_ON_GREY: Rgb = [128, 146, 172];

/// What is under a field's colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Under {
    /// A neutral grey ([`FieldLook::no_value`]), which cells without a
    /// value show.
    #[default]
    Neutral,
    /// The geometry's colours, which cells without a value keep.
    Geometry,
}

/// How a field is drawn: the colours of its values, how much of a pixel
/// they take and what is under them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldLook {
    pub scale: Scale,
    /// How much of a pixel the field's colour takes (0 to 1).
    pub opacity: f32,
    pub under: Under,
    /// The colour of values below the scale (on a log scale, zero and
    /// negative ones too), instead of its low end's.
    pub below: Option<Rgb>,
    /// The colour of values above the scale, instead of its high end's.
    pub above: Option<Rgb>,
    /// Only values from the first to the second (each end included; none,
    /// no bound) are drawn: the others are drawn as no value.
    pub threshold: [Option<f64>; 2],
}

impl FieldLook {
    /// A scale fitted to `field`'s values ([`FieldLook::fit`]), fully
    /// opaque.
    pub fn fitted(field: &Field) -> FieldLook {
        let mut look = FieldLook {
            scale: Scale::linear(0.0, 1.0),
            opacity: 1.0,
            under: Under::Neutral,
            below: None,
            above: None,
            threshold: [None; 2],
        };
        look.fit(field);
        look
    }

    /// Fit the scale to `field`'s values, keeping its ramp: logarithmic when
    /// none is negative and the positive ones span two decades or more
    /// (zeros, nothing scored, then fall below it and take its low end's
    /// colour), linear otherwise ([`Scale::auto`]). Unchanged if the field
    /// has no finite value.
    pub fn fit(&mut self, field: &Field) {
        let finite = || field.numbers().map(f64::from).filter(|x| x.is_finite());
        let signed = finite().any(|x| x < 0.0);
        let log = !signed && Scale::auto(finite(), true).is_some_and(|s| s.hi >= 100.0 * s.lo);
        if let Some(s) = Scale::auto(finite(), log) {
            self.scale = Scale {
                ramp: self.scale.ramp,
                bands: self.scale.bands,
                ..s
            };
        }
    }

    /// The colour of value `v`.
    pub fn colour(&self, v: f64) -> Rgb {
        self.scale.colour(v)
    }

    /// The grey of no value: [`NO_VALUE`], or on the grey ramp (which runs
    /// through it) [`NO_VALUE_ON_GREY`].
    pub fn no_value(&self) -> Rgb {
        match self.scale.ramp {
            crate::scale::Ramp::Grey => NO_VALUE_ON_GREY,
            _ => NO_VALUE,
        }
    }

    /// What is under the field where the geometry's colour is `geometry`:
    /// that, or the neutral grey.
    pub fn base(&self, geometry: Rgb) -> Rgb {
        match self.under {
            Under::Neutral => self.no_value(),
            Under::Geometry => geometry,
        }
    }

    /// Whether `v` is drawn: finite, and within the threshold.
    pub fn shows(&self, v: f64) -> bool {
        let [lo, hi] = self.threshold;
        v.is_finite() && lo.is_none_or(|lo| v >= lo) && hi.is_none_or(|hi| v <= hi)
    }

    /// Whether `v` lies below the scale (on a log scale, any value not
    /// positive too) or above it.
    pub fn outside(&self, v: f64) -> std::cmp::Ordering {
        let s = &self.scale;
        if v < s.lo || (s.log && !(v > 0.0)) {
            std::cmp::Ordering::Less
        } else if v > s.hi {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }

    /// The colour of value `v` as drawn: none where it is not shown
    /// ([`FieldLook::shows`]), [`FieldLook::below`] or
    /// [`FieldLook::above`] outside the scale when they are set, else the
    /// scale's.
    pub fn value_colour(&self, v: f64) -> Option<Rgb> {
        if !self.shows(v) {
            return None;
        }
        let out = match self.outside(v) {
            std::cmp::Ordering::Less => self.below,
            std::cmp::Ordering::Greater => self.above,
            std::cmp::Ordering::Equal => None,
        };
        Some(out.unwrap_or_else(|| self.colour(v)))
    }

    /// The colour of a cell whose geometry's colour is `geometry` and whose
    /// value is `v` (none, not finite or outside the threshold: no value):
    /// the value's colour ([`FieldLook::value_colour`]) over
    /// [`FieldLook::base`], `opacity` of the way, or the base alone.
    pub fn over(&self, geometry: Rgb, v: Option<f64>) -> Rgb {
        let base = self.base(geometry);
        match v.and_then(|v| self.value_colour(v)) {
            Some(c) => mix(base, c, self.opacity),
            None => base,
        }
    }
}

/// A field as drawn: its numbers (shared, never copied to change the
/// look) and its look.
#[derive(Clone, Debug)]
pub struct Overlay {
    pub field: Arc<Field>,
    pub look: FieldLook,
}

impl Overlay {
    /// `field` with a look fitted to it ([`FieldLook::fitted`]).
    pub fn new(field: Arc<Field>) -> Overlay {
        let look = FieldLook::fitted(&field);
        Overlay { field, look }
    }
}

/// `a` moved `t` of the way to `b` (rounded to the nearest level).
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| ((1.0 - t) * a[i] as f32 + t * b[i] as f32).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aci_tallies_geom::Geometry;

    #[test]
    fn fitted_scales() {
        let g = Geometry::Rectangular {
            x_bounds: vec![-5.0, 0.0, 5.0],
            y_bounds: vec![-5.0, 5.0],
            z_bounds: vec![-5.0, 5.0],
            transform: None,
        };
        let f = Field::mesh("flux", "", g.clone(), vec![1.0, 100.0], None).unwrap();
        let look = FieldLook::fitted(&f);
        assert!(look.scale.log && look.opacity == 1.0);
        let f = Field::mesh("t", "", g, vec![-1.0, 100.0], None).unwrap();
        assert!(!FieldLook::fitted(&f).scale.log);
        // Zeros (nothing scored) do not make a log scale linear.
        let g3 = Geometry::Rectangular {
            x_bounds: vec![-5.0, 0.0, 5.0, 10.0],
            y_bounds: vec![-5.0, 5.0],
            z_bounds: vec![-5.0, 5.0],
            transform: None,
        };
        let f = Field::mesh("flux", "", g3, vec![0.0, 1.0, 100.0], None).unwrap();
        let look = FieldLook::fitted(&f);
        assert!(look.scale.log && look.scale.lo > 0.0);
        assert_eq!(
            look.over([9, 9, 9], Some(0.0)),
            look.scale.colour(look.scale.lo)
        );
    }

    #[test]
    fn cells_without_a_value_are_grey_under_a_neutral_base() {
        let mut look = FieldLook {
            scale: Scale::linear(0.0, 1.0),
            opacity: 1.0,
            under: Under::Neutral,
            below: None,
            above: None,
            threshold: [None; 2],
        };
        let cell = [200, 30, 30];
        assert_eq!(look.over(cell, None), NO_VALUE);
        assert_eq!(look.over(cell, Some(f64::NAN)), NO_VALUE);
        assert_eq!(look.over(cell, Some(1.0)), look.colour(1.0));
        // Below the scale: its low end, not "no value".
        assert_eq!(look.over(cell, Some(-5.0)), look.colour(0.0));
        look.opacity = 0.5;
        assert_eq!(
            look.over(cell, Some(1.0)),
            mix(NO_VALUE, look.colour(1.0), 0.5)
        );
        look.under = Under::Geometry;
        assert_eq!(look.over(cell, None), cell);
        assert_eq!(look.over(cell, Some(1.0)), mix(cell, look.colour(1.0), 0.5));
        // On the grey ramp, whose middle is the grey, a tinted one, unlike
        // any of its colours.
        look.under = Under::Neutral;
        look.scale.ramp = crate::scale::Ramp::Grey;
        assert_eq!(look.over(cell, None), NO_VALUE_ON_GREY);
        assert!((0..=255).all(|k| look.scale.ramp.at(k as f64 / 255.0) != NO_VALUE_ON_GREY));
    }

    #[test]
    fn thresholds_and_colours_outside_the_scale() {
        let g = Geometry::Rectangular {
            x_bounds: vec![0.0, 1.0],
            y_bounds: vec![0.0, 1.0],
            z_bounds: vec![0.0, 1.0],
            transform: None,
        };
        let f = Field::mesh("f", "", g, vec![1.0], None).unwrap();
        let mut look = FieldLook::fitted(&f);
        look.scale = Scale::log(1.0, 100.0);
        let cell = [1, 2, 3];
        // Outside the scale: its ends' colours, or those asked for.
        assert_eq!(look.over(cell, Some(0.0)), look.scale.colour(1.0));
        assert_eq!(look.over(cell, Some(1e9)), look.scale.colour(100.0));
        look.below = Some([0, 0, 255]);
        look.above = Some([255, 0, 0]);
        assert_eq!(look.over(cell, Some(0.0)), [0, 0, 255]);
        assert_eq!(look.over(cell, Some(0.5)), [0, 0, 255]);
        assert_eq!(look.over(cell, Some(1e9)), [255, 0, 0]);
        assert_eq!(look.over(cell, Some(10.0)), look.scale.colour(10.0));
        // A threshold: the rest is no value (one bound, or both).
        look.threshold = [Some(5.0), None];
        assert_eq!(look.over(cell, Some(4.9)), look.no_value());
        assert_eq!(look.over(cell, Some(5.0)), look.scale.colour(5.0));
        assert_eq!(look.over(cell, Some(1e9)), [255, 0, 0]);
        look.threshold = [Some(5.0), Some(50.0)];
        assert_eq!(look.over(cell, Some(60.0)), look.no_value());
    }

    #[test]
    fn mixing() {
        assert_eq!(mix([0, 0, 0], [255, 100, 10], 1.0), [255, 100, 10]);
        assert_eq!(mix([0, 0, 0], [255, 100, 10], 0.5), [128, 50, 5]);
        assert_eq!(mix([9, 9, 9], [255, 100, 10], 0.0), [9, 9, 9]);
    }
}
