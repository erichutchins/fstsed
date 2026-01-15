//! Private utilities and shared constants.

use crate::error::{FstSedError, Result};
use camino::Utf8Path;
use fst::raw::Fst;
use memmap2::Mmap;
use std::fs::File;

/// Default buffer size for I/O operations (64 KiB).
pub const BUFFERSIZE: usize = 64 * 1024;

/// Sentinel byte used to separate keys from values in the FST.
pub const SENTINEL: u8 = 0;

const IS_WORD_BYTE_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    table[b'_' as usize] = true;
    let mut b = b'0';
    while b <= b'9' {
        table[b as usize] = true;
        b += 1;
    }
    b = b'A';
    while b <= b'Z' {
        table[b as usize] = true;
        b += 1;
    }
    b = b'a';
    while b <= b'z' {
        table[b as usize] = true;
        b += 1;
    }
    table
};

/// Returns `true` if the given byte is an ASCII word character.
#[inline]
#[must_use]
pub fn is_word_byte(b: u8) -> bool {
    IS_WORD_BYTE_TABLE[b as usize]
}

/// Returns `true` if the given character is a word character.
#[inline]
#[must_use]
pub fn is_word_char(c: char) -> bool {
    if (c as u32) < 128 {
        IS_WORD_BYTE_TABLE[c as usize]
    } else {
        c.is_alphanumeric()
    }
}

/// Memory-maps an FST file from disk.
///
/// # Safety
///
/// The caller must ensure the file is not modified while the returned
/// FST is in use.
#[inline]
pub unsafe fn mmap_fst(path: &Utf8Path) -> Result<Fst<Mmap>> {
    let mmap = Mmap::map(&File::open(path).map_err(FstSedError::FstMmap)?)
        .map_err(FstSedError::FstMmap)?;
    let fst = fst::raw::Fst::new(mmap).map_err(FstSedError::FstParse)?;
    Ok(fst)
}
