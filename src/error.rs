//! Error types for the fstsed library.
//!
//! This module provides structured error types using `thiserror` for
//! better error handling ergonomics for library consumers.

use std::io;
use thiserror::Error;

/// The main error type for fstsed operations.
#[derive(Error, Debug)]
pub enum FstSedError {
    /// Error opening or reading an FST file.
    #[error("failed to open FST file: {0}")]
    FstOpen(#[source] io::Error),

    /// Error memory-mapping an FST file.
    #[error("failed to memory-map FST file: {0}")]
    FstMmap(#[source] io::Error),

    /// Error parsing FST data structure.
    #[error("failed to parse FST data: {0}")]
    FstParse(#[source] fst::Error),

    /// Error building an FST database.
    #[error("FST build failed: {0}")]
    Build(#[source] io::Error),

    /// Error parsing JSON input during FST building.
    #[error("JSON parse error: {0}")]
    JsonParse(#[source] serde_json::Error),

    /// Error writing FST data during build.
    #[error("FST write error: {0}")]
    FstWrite(#[source] fst::Error),
}

/// A specialized `Result` type for fstsed operations.
pub type Result<T> = std::result::Result<T, FstSedError>;

// Automatic conversions from common error types

impl From<io::Error> for FstSedError {
    fn from(err: io::Error) -> Self {
        FstSedError::Build(err)
    }
}

impl From<fst::Error> for FstSedError {
    fn from(err: fst::Error) -> Self {
        FstSedError::FstParse(err)
    }
}

impl From<serde_json::Error> for FstSedError {
    fn from(err: serde_json::Error) -> Self {
        FstSedError::JsonParse(err)
    }
}
