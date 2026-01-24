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

/// Compile-time lookup table for ASCII word characters (`[a-zA-Z0-9_]`).
///
/// A single array index operation is faster than multiple range checks.
/// The table is computed at compile time, so there's no runtime cost.
///
/// Note: We use `while` loops instead of `for` or iterators because this is
/// a `const` context—`for` loops and iterator methods aren't const-stable yet.
const IS_WORD_BYTE_TABLE: [bool; 128] = {
    let mut table = [false; 128];

    // Underscore
    table[b'_' as usize] = true;

    // Digits: 0-9
    let mut b = b'0';
    while b <= b'9' {
        table[b as usize] = true;
        b += 1;
    }

    // Uppercase: A-Z
    b = b'A';
    while b <= b'Z' {
        table[b as usize] = true;
        b += 1;
    }

    // Lowercase: a-z
    b = b'a';
    while b <= b'z' {
        table[b as usize] = true;
        b += 1;
    }

    table
};

/// Returns `true` if the given byte is an ASCII word character (`[a-zA-Z0-9_]`).
///
/// Used for fast word boundary detection when processing ASCII text.
/// A word boundary exists where a word character meets a non-word character.
#[inline]
#[must_use]
pub fn is_word_byte(b: u8) -> bool {
    b < 128 && IS_WORD_BYTE_TABLE[b as usize]
}

/// Returns `true` if the given character is a word character.
///
/// Word characters are used to detect word boundaries (similar to regex `\b`):
/// - ASCII: `[a-zA-Z0-9_]` — fast lookup table
/// - Unicode: any alphanumeric character per Unicode standard
#[inline]
#[must_use]
pub fn is_word_char(c: char) -> bool {
    if (c as u32) < 128 {
        // ASCII fast path: single table lookup
        IS_WORD_BYTE_TABLE[c as usize]
    } else {
        // Unicode fallback: delegate to std library
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
