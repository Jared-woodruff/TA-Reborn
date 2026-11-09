//! Clean-room readers for the Total Annihilation (1997) data formats.
//!
//! All parsers take whole-file byte slices (as produced by [`hpi::HpiArchive::read`] or the
//! VFS) and return owned, engine-friendly structures. They are written from the publicly
//! documented community specifications of the formats; no original game code is used.

mod bin;
pub mod cob;
pub mod fnt;
pub mod gaf;
pub mod hpi;
pub mod image;
pub mod model3do;
pub mod pcx;
pub mod tdf;
pub mod tnt;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("unexpected end of data at offset {offset} (needed {needed} bytes)")]
    Eof { offset: usize, needed: usize },
    #[error("invalid {what}: {detail}")]
    Invalid { what: &'static str, detail: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl FormatError {
    pub(crate) fn invalid(what: &'static str, detail: impl Into<String>) -> Self {
        FormatError::Invalid { what, detail: detail.into() }
    }
}

pub type Result<T> = std::result::Result<T, FormatError>;
