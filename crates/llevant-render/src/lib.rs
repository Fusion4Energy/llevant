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
