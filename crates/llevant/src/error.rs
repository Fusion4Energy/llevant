//! Errors of the public API.

use std::fmt;
use std::path::PathBuf;

use crate::trace::Line;

/// What can go wrong in a call to llevant. Problems in the MCNP input are
/// not errors: they are [`Diagnostic`](crate::Diagnostic)s, and the cells
/// they affect are compiled as unsupported.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A file could not be read.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A line of a batch (`index`) has a range that is not finite and
    /// ordered, or a direction that is not a unit vector.
    Line { index: usize, line: Line },
    /// No cell has this MCNP number.
    NoCell(i64),
    /// No universe has this MCNP number.
    NoUniverse(i64),
    /// An argument outside what the call accepts; the message says which.
    Invalid(String),
    /// The GPU renderer: no adapter, or the model does not fit.
    Gpu(String),
    /// Stopped through its [`Progress`](crate::Progress).
    Cancelled,
    /// Bytes that are not a model saved by this build of the engine
    /// ([`Model::read`](crate::Model::read)), or a model that could not be
    /// written; the message says why.
    Format(String),
}

/// `Result` with llevant's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Line { index, line } => {
                let what =
                    if line.t_min.is_finite() && line.t_max.is_finite() && line.t_min <= line.t_max
                    {
                        "direction is not a unit vector"
                    } else {
                        "range is not finite and ordered"
                    };
                write!(f, "line {index}: {what}: {line:?}")
            }
            Error::NoCell(id) => write!(f, "no cell {id}"),
            Error::NoUniverse(u) => write!(f, "no universe {u}"),
            Error::Invalid(m) => f.write_str(m),
            Error::Gpu(m) => write!(f, "GPU: {m}"),
            Error::Cancelled => f.write_str("cancelled"),
            Error::Format(m) => write!(f, "saved model: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
