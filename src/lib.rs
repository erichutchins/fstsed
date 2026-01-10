pub mod build;
pub mod cli;
pub mod fstsed;
pub mod jsonquotes;

pub const BUFFERSIZE: usize = 64 * 1024;

/// Compile-time lookup table for ASCII word characters.
/// Word chars are: a-z, A-Z, 0-9, _
/// This matches ripgrep's `utf8::is_word_byte` implementation.
const IS_WORD_BYTE_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    // underscore
    table[b'_' as usize] = true;
    // 0-9
    let mut b = b'0';
    while b <= b'9' {
        table[b as usize] = true;
        b += 1;
    }
    // A-Z
    b = b'A';
    while b <= b'Z' {
        table[b as usize] = true;
        b += 1;
    }
    // a-z
    b = b'a';
    while b <= b'z' {
        table[b as usize] = true;
        b += 1;
    }
    table
};

/// Fast check if a byte is an ASCII word character.
/// This is the hot-path function used in the main search loop.
#[inline(always)]
pub fn is_word_byte(b: u8) -> bool {
    IS_WORD_BYTE_TABLE[b as usize]
}

/// is_word_char checks if a character is a word character.
/// For ASCII (< 256), uses the fast lookup table.
/// For Unicode, falls back to is_alphanumeric().
#[inline]
pub fn is_word_char(c: char) -> bool {
    if (c as u32) < 128 {
        IS_WORD_BYTE_TABLE[c as usize]
    } else {
        c.is_alphanumeric()
    }
}
