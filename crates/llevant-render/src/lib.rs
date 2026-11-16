//! Images of llevant's MCNP models: the colours of slices and renders, and
//! physically based shading.
//!
//! The engine (`llevant`) answers geometry questions: labels along slice
//! rows ([`llevant::Model::slice`]), first hits of camera rays
//! ([`llevant::Model::render`]). `llevant-scene` holds what frontends
//! build from a model (material looks, fields, hidden cells). This crate
//! turns them into 8-bit RGB images, as the `llevant` command, the viewer
//! and the Python bindings draw them:
//!
//! - [`colours`]: a colour per cell by cell, material or universe number
//!   or density ([`colours::Paint`], which every image is coloured by),
//!   and images of slices.
//! - [`scale`]: numbers to colours (densities, fields) and colour bars.
//! - [`legend`]: legends of fields: ticks, their numbers, layout, and the
//!   legend drawn into an image.
//! - [`field`]: how a field is drawn over slices and renders
//!   ([`field::FieldLook`], [`field::Overlay`]).
//! - [`shading`]: render images: flat colours, or realistic ones lit
//!   physically from each pixel's hit.
//! - [`labels`]: where to write cell and surface numbers on slices.
//! - [`ghost`]: ghost shells over renders.
//! - [`png`]: PNG files.
//!
//! A slice, volumes and a realistic image of a model:
//!
//! ```no_run
//! use llevant::{Axis, Camera, Depth, Model, Options, Scope, Slice, Visibility, VolumeGrid};
//! use llevant_render::{colours::{ColourBy, Paint}, png::write_png, shading};
//! use llevant_scene::materials::Looks;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let model = Model::load("model.mcnp", &Options::default(), None)?;
//!     let region = model.extent(Scope::Contents, 128); // measured box around the contents
//!     let slice = Slice::axis(Axis::Z, 37.0, &region, 1000); // the plane z = 37
//!     let ids = slice.pixels(&model.slice(&slice, Depth::All)?); // label id per pixel
//!     println!("{} pixels", ids.len());
//!
//!     let vols = model.volumes(&VolumeGrid::seeded(region, 2.0, 1), None)?;
//!     for i in vols.problems() {
//!         println!("{}: {} cm3", model.describe(&vols.labels[i]), vols.volumes[i].estimate);
//!     }
//!
//!     // Material looks are a value of their own: the model never changes.
//!     let looks = Looks::new(&model, &[(12, "copper roughness=0.2".parse()?)])?;
//!     let cam = Camera::fit(&region, [0.6, 0.7, -0.4], 800, 600);
//!     let hits = model.render(&cam, &Visibility::solid(&model))?;
//!     let probes = shading::Probe::from_hits(&model, &hits);
//!     let paint = Paint::new(&model, &looks, ColourBy::Realistic);
//!     let rgb = shading::render_image(&paint, &probes, &cam)?;
//!     write_png("view.png".as_ref(), 800, 600, &rgb)?;
//!     Ok(())
//! }
//! ```

pub mod colours;
pub mod field;
pub mod ghost;
pub mod labels;
pub mod legend;
pub mod png;
pub mod scale;
pub mod shading;

/// The README's Rust examples, compiled as doctests so that they stay true
/// to the API (`no_run`: they need a model file).
#[doc = include_str!("../../../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
