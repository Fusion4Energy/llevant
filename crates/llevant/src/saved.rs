//! Compiled models as bytes ([`Model::save`], [`Model::read`],
//! [`Model::from_bytes`]): what frontends keep in a disk cache, so that
//! loading an input compiled before takes a read instead of a compile.
//!
//! The bytes are a header (magic, [`FORMAT_VERSION`], [`ENGINE_HASH`]) and
//! then sections, each a little-endian `u64` length and that many bytes of
//! bitcode:
//!
//! - the model without its octrees;
//! - the number of universes with an octree;
//! - for each of them: its index, then the octree's root box and its arrays,
//!   each array as its length and then pieces of at most [`PIECE`] items.
//!
//! Sections are made and written one by one, so saving needs about one
//! piece more than the model itself, not a second copy of it; reading
//! decodes from a slice (a memory map of the file, say) into the model's
//! own arrays. Only models saved by this very build of the engine are read
//! back: another build may compile differently, and there is no migration.

use std::io::{Read, Write};
use std::time::Instant;

use serde::{de::DeserializeOwned, Serialize};

use crate::error::{Error, Result};
use crate::grid::Octree;
use crate::model::Model;

const MAGIC: &[u8; 8] = b"llevant\0";

/// Version of the layout [`Model::save`] writes (the header and the
/// sections; what is inside them is tied to [`ENGINE_HASH`]).
pub const FORMAT_VERSION: u32 = 3;

/// Digest of this build of the engine: its source, version and dependency
/// versions (`build.rs`). Saved models carry it, and a model saved by a
/// build with another digest is not read. Caches put it in their keys.
pub const ENGINE_HASH: &str = env!("LLEVANT_SOURCE_HASH");

/// Items of an array per section: 4 M `u32` words make 16 MB pieces, so
/// writing never holds more than one piece's bytes besides the model.
const PIECE: usize = 1 << 22;

fn format(what: impl std::fmt::Display) -> Error {
    Error::Format(what.to_string())
}

/// Writes sections to `w`.
struct Sections<'w, W: Write>(&'w mut W);

impl<W: Write> Sections<'_, W> {
    fn value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        let bytes = bitcode::serialize(v).map_err(format)?;
        self.0
            .write_all(&(bytes.len() as u64).to_le_bytes())
            .map_err(format)?;
        self.0.write_all(&bytes).map_err(format)
    }

    fn array<T: Serialize>(&mut self, a: &[T]) -> Result<()> {
        self.value(&(a.len() as u64))?;
        for piece in a.chunks(PIECE) {
            self.value(piece)?;
        }
        Ok(())
    }
}

/// Reads sections from a slice.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(format("saved model is cut short"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn value<T: DeserializeOwned>(&mut self) -> Result<T> {
        let len = u64::from_le_bytes(self.take(8)?.try_into().unwrap());
        let len = usize::try_from(len).map_err(format)?;
        bitcode::deserialize(self.take(len)?).map_err(format)
    }

    fn array<T: DeserializeOwned>(&mut self) -> Result<Vec<T>> {
        let len = usize::try_from(self.value::<u64>()?).map_err(format)?;
        // All at once, so the pieces never make it grow (a doubling would
        // be a second copy); a bad length fails here, not by aborting.
        let mut out: Vec<T> = Vec::new();
        out.try_reserve_exact(len).map_err(format)?;
        while out.len() < len {
            let piece: Vec<T> = self.value()?;
            if piece.is_empty() || out.len() + piece.len() > len {
                return Err(format("saved model has a bad array"));
            }
            out.extend(piece);
        }
        Ok(out)
    }
}

impl Model {
    /// Write the compiled model to `w`, to be read back with [`Model::read`]
    /// or [`Model::from_bytes`] by the same build of the engine.
    pub fn save(&self, w: &mut impl Write) -> Result<()> {
        if !self.is_complete() {
            return Err(Error::Invalid(
                "a staged model cannot be saved: complete it first".into(),
            ));
        }
        let hash = ENGINE_HASH.as_bytes();
        let mut head = Vec::with_capacity(8 + 4 + 4 + hash.len());
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        head.extend_from_slice(&(hash.len() as u32).to_le_bytes());
        head.extend_from_slice(hash);
        w.write_all(&head).map_err(format)?;
        let mut s = Sections(w);
        // `Unit::grid` is not serialized: the octrees follow, one by one.
        s.value(self)?;
        let grids: Vec<(u32, &std::sync::Arc<Octree>)> = self
            .units
            .iter()
            .enumerate()
            .filter_map(|(i, u)| u.grid.as_ref().map(|g| (i as u32, g)))
            .collect();
        s.value(&(grids.len() as u64))?;
        for (i, g) in grids {
            s.value(&i)?;
            s.value(&g.root)?;
            s.array(&g.nodes)?;
            s.array(&g.items)?;
            s.array(&g.progs)?;
            s.array(&g.arena)?;
        }
        Ok(())
    }

    /// Read a model written by [`Model::save`]. Fails with [`Error::Format`]
    /// on bytes that are not a model saved by this build of the engine.
    /// Reads all the bytes first: [`Model::from_bytes`] on a memory map of
    /// a file needs no copy of them.
    pub fn read(r: &mut impl Read) -> Result<Model> {
        let t = Instant::now();
        let mut bytes = Vec::new();
        r.read_to_end(&mut bytes).map_err(format)?;
        let mut m = Model::from_bytes(&bytes)?;
        m.timings = vec![("saved model", t.elapsed().as_secs_f64())];
        Ok(m)
    }

    /// The model saved in `bytes` by [`Model::save`]. Fails with
    /// [`Error::Format`] on bytes that are not a model saved by this build
    /// of the engine, or not all of one.
    pub fn from_bytes(bytes: &[u8]) -> Result<Model> {
        let t = Instant::now();
        let mut r = Reader(bytes);
        if r.take(8).ok() != Some(&MAGIC[..]) {
            return Err(format("not a saved llevant model"));
        }
        let version = u32::from_le_bytes(r.take(4)?.try_into().unwrap());
        if version != FORMAT_VERSION {
            return Err(format(format_args!(
                "saved model format {version}, this build reads {FORMAT_VERSION}"
            )));
        }
        let len = u32::from_le_bytes(r.take(4)?.try_into().unwrap()) as usize;
        if len != ENGINE_HASH.len() || r.take(len)? != ENGINE_HASH.as_bytes() {
            return Err(format("saved by another build of llevant"));
        }
        let mut m: Model = r.value()?;
        let grids: u64 = r.value()?;
        for _ in 0..grids {
            let i: u32 = r.value()?;
            let root = r.value()?;
            let (nodes, items, progs, arena) = (r.array()?, r.array()?, r.array()?, r.array()?);
            let unit = m
                .units
                .get_mut(i as usize)
                .filter(|u| u.grid.is_none())
                .ok_or_else(|| format("saved model has an octree of no universe"))?;
            unit.grid = Some(std::sync::Arc::new(Octree {
                root,
                nodes,
                items,
                progs,
                arena,
            }));
        }
        if !r.0.is_empty() {
            return Err(format("saved model has bytes after its end"));
        }
        m.timings = vec![("saved model", t.elapsed().as_secs_f64())];
        Ok(m)
    }
}
