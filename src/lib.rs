use std::sync::LazyLock;

pub mod build;
pub mod cli;
pub mod fstsed;
pub mod jsonquotes;

pub const BUFFERSIZE: usize = 64 * 1024;

static IS_WORD_TABLE: LazyLock<[bool; 256]> = LazyLock::new(|| {
    let mut table = [false; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let c = i as u8 as char;
        if c.is_alphanumeric() || c == '_' {
            *slot = true;
        }
    }
    table
});

/// is_word_char checks if a byte is a word character (per Unicode)
/// This is used to validate match boundaries without a full regex engine call.
#[inline]
pub fn is_word_char(c: char) -> bool {
    if (c as u32) < 256 {
        IS_WORD_TABLE[c as usize]
    } else {
        c.is_alphanumeric()
    }
}
