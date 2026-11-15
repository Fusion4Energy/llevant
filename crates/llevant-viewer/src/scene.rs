//! The scene: what the user chose to show of the model. Colours, hidden
//! cells, the clip plane, effects, looks of materials, pins, and the FMESH
//! meshes laid out over it. It is the
//! part of the view that commands edit and undo ([`crate::command`]), and the
//! part of the view file that is not a camera or a slice
//! ([`crate::ViewState`]).

use std::collections::BTreeMap;

use llevant::{Model, Result, Visibility};
use llevant_scene::hiding::Hiding;
use llevant_scene::index::ModelIndex;
use llevant_scene::materials::{Looks, Spec};
use serde::{Deserialize, Serialize};

use crate::mesh::MeshState;
use crate::state::{ClipState, Colouring, CutState, Effects, FieldState};

/// A point marked by the user, shown with a banner in both views.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub point: [f64; 3],
    /// The user's note, shown as the banner's title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A rule kept with the view, by name (the Find panel's Saved).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedRule {
    pub name: String,
    /// In the form `llevant_scene::query::Query` reads.
    pub rule: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scene {
    /// Realistic colours are lit physically, the others flat.
    pub colour_by: Colouring,
    /// Looks of materials for realistic colours, by MCNP material number:
    /// `"steel"`, `"copper roughness=0.2"`, `"colour=#806040 metalness=0"`
    /// (as `llevant render --look`).
    #[serde(deserialize_with = "material_keys")]
    pub materials: BTreeMap<i64, String>,
    /// What the 3D view hides (void cells, cells, universes, materials):
    /// the rules `llevant render` and the Python bindings use too. In the
    /// view file its fields sit at the top level (`hide_void`,
    /// `hidden_cells`...).
    #[serde(flatten)]
    pub hiding: Hiding,
    /// Plane cutting away the half-space its normal points into (3D).
    pub clip: Option<ClipState>,
    /// More cuts of the 3D view: planes at any angle, wedges, boxes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cuts: Vec<CutState>,
    /// The cuts turned off (indices into `cuts`, ascending): kept, but
    /// cutting nothing and not drawn.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cuts_off: Vec<usize>,
    pub effects: Effects,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<Pin>,
    /// Numbers drawn over the cells (results of a run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<FieldState>,
    /// FMESH tallies being laid out (in Tally).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub meshes: Vec<MeshState>,
    /// Rules saved by name (the Find panel).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<SavedRule>,
}

/// The material map with its numbers as JSON keys (strings): inside the
/// flattened view file serde no longer parses them as numbers itself.
fn material_keys<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<i64, String>, D::Error> {
    let raw = BTreeMap::<String, String>::deserialize(d)?;
    raw.into_iter()
        .map(|(k, v)| {
            let id = k
                .trim()
                .trim_start_matches(['m', 'M'])
                .parse()
                .map_err(|_| serde::de::Error::custom(format!("material number `{k}`")))?;
            Ok((id, v))
        })
        .collect()
}

/// What differs between two scenes, for what depends on each part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub colouring: bool,
    pub looks: bool,
    pub visibility: bool,
    pub effects: bool,
    pub pins: bool,
    /// The field's numbers (another file, tally or bin; or none).
    pub field_source: bool,
    /// How the field is drawn (its scale, opacity, whether shown).
    pub field_look: bool,
}

impl Changes {
    pub fn between(a: &Scene, b: &Scene) -> Changes {
        Changes {
            colouring: a.colour_by != b.colour_by,
            looks: a.materials != b.materials,
            visibility: a.hiding != b.hiding,
            effects: a.effects != b.effects,
            pins: a.pins != b.pins,
            field_source: a.field.as_ref().map(|f| &f.source)
                != b.field.as_ref().map(|f| &f.source),
            field_look: a.field != b.field,
        }
    }
}

impl Scene {
    /// What the 3D view cuts away: the clip plane and the cuts.
    pub fn cutaway(&self) -> llevant::Cutaway {
        let mut c: llevant::Cutaway = self.clip.map(llevant::Clip::from).into();
        for (_, cut) in self.cuts_on() {
            c = c.union(cut.cutaway());
        }
        c
    }

    /// Whether cut `k` cuts (is not turned off).
    pub fn cut_on(&self, k: usize) -> bool {
        !self.cuts_off.contains(&k)
    }

    /// The cuts that cut, with their indices.
    pub fn cuts_on(&self) -> impl Iterator<Item = (usize, &CutState)> {
        self.cuts
            .iter()
            .enumerate()
            .filter(|(k, _)| self.cut_on(*k))
    }

    /// The looks of `m`'s materials with the specs of the scene.
    pub fn looks(&self, m: &Model) -> Result<Looks> {
        Looks::new(m, &self.specs()?)
    }

    /// The errors of [`Scene::looks`] (bad specs, unknown materials)
    /// without its work.
    pub fn check_looks(&self, m: &Model) -> Result<()> {
        Looks::check(m, &self.specs()?).map(|_| ())
    }

    fn specs(&self) -> Result<Vec<(i64, Spec)>> {
        self.materials
            .iter()
            .map(|(&id, s)| Ok((id, s.parse::<Spec>()?)))
            .collect()
    }

    /// The cells shown in 3D ([`Hiding::visibility_with`]). Unknown
    /// numbers are an error.
    pub fn visibility(&self, m: &Model, x: &ModelIndex) -> Result<Visibility> {
        self.hiding.visibility_with(m, x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_name_what_differs() {
        let a = Scene::default();
        assert_eq!(Changes::between(&a, &a), Changes::default());
        let mut b = a.clone();
        b.hiding.cells = vec![3];
        b.pins.push(Pin {
            point: [0.0; 3],
            note: String::new(),
        });
        let c = Changes::between(&a, &b);
        assert!(c.visibility && c.pins && !c.colouring && !c.looks);
    }
}
