//! What frontends of llevant (the `llevant` command, the viewer, the Python
//! bindings) share about an MCNP model, beyond the engine's geometry
//! answers and before any pixel:
//!
//! - [`store`]: loading models through a disk cache of compiled ones.
//! - [`source`]: the input as written (the text and line of each card).
//! - [`index`]: cells by material, universe and fill, and the universe
//!   tree ([`index::ModelIndex`]).
//! - [`query`]: rules that pick cells (`c 10-20`, `m 3`, `u 5`,
//!   `label "BSV"`...).
//! - [`labels`]: cells by the `$` comments of their cards, read from the
//!   input when a rule needs them.
//! - [`bounds`]: where a set of cells is: the global box of each instance
//!   and their union.
//! - [`hiding`]: which cells 3D views hide, as the engine's
//!   [`llevant::Visibility`].
//! - [`materials`]: each `Mn` card sorted by composition into categories
//!   (steel, concrete, water...) and given an appearance; [`materials::Looks`]
//!   holds them for one model, with the user's overrides.
//! - [`field`]: numbers per cell, cell instance or mesh voxel (results of
//!   a transport run), made in code or read from CSV and (feature
//!   `tallies`) MCNP tally files.
//!
//! All of it is built from a model and owned by the caller: the model
//! stays read-only. Drawing (colours, scales, shading) is `llevant-render`.

pub mod bounds;
pub mod field;
pub mod hiding;
pub mod index;
pub mod labels;
pub mod materials;
pub mod query;
pub mod source;
pub mod store;
