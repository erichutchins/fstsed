use itermore::IterArrayChunks;
use memchr::memchr2_iter;
use memchr::Memchr2;

/// Identifies the structural double quotation marks bounding strings in json text.
/// Searches for all double quotes and backslashes simultaneously using memchr2. Then, for each
/// match, record state to determine if the double quote was escaped by the backslash or is a
/// real structural quote. The iterator returns a flat, linear feed of structural quote indices
pub struct JsonQuotes<'a> {
    haystack: &'a [u8],
    iter: Memchr2<'a>,
    pending_escape: Option<usize>,
}

impl<'a> JsonQuotes<'a> {
    pub fn new(haystack: &'a [u8]) -> Self {
        Self {
            haystack,
            iter: memchr2_iter(b'"', b'\\', haystack),
            pending_escape: None,
        }
    }
}

impl Iterator for JsonQuotes<'_> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(index) = self.iter.next() {
            // Check if the current character is escaped by a backslash found in the previous
            // iteration. If the previous match was at the immediately preceding index,
            // then this character is escaped and should be ignored.
            if self.pending_escape == Some(index.wrapping_sub(1)) {
                // This is a true escaped, so skip it
                self.pending_escape = None;
                continue;
            }

            // If we reach here, the current `index` was NOT escaped by the previous `\`
            // Now, determine if it's a structural quote or a new potential escape.
            if self.haystack[index] == b'"' {
                // we have a structural quote. reset escape counter
                // and return the index position
                self.pending_escape = None;
                return Some(index);
            } else {
                // we have a \
                // mark it as a potential escape for the NEXT match
                self.pending_escape = Some(index);
            }
        }
        None
    }
}

/// Isolate just the ranges of strings in json to avoid deserializing the entire structure. Uses
/// memchr2 to find all doublequotes and backslashes simultaneously and tracks state to determine
/// when the backslashes escape the quotes.
///
/// This function returns an iterator of (start, end) tuples of the string ranges. *Note* the
/// indices include the quotation marks themselves!
#[inline]
pub fn jsonquotes_range_iter<'a>(
    haystack: &'a [u8],
) -> impl Iterator<Item = (usize, usize)> + 'a {
    // Rather than have JsonQuotes bother with knowing if a quote is an open or close,
    // the indices come to us in a flat series and we just iterate in chunks of
    // two giving us each start, stop index. We add 1 so when this tuple is used to
    // retrieve the str, both open and close quotes are themselves included
    IterArrayChunks::array_chunks::<2>(JsonQuotes::new(haystack)).map(move |[a, b]| (a, b + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_0_escape() {
        // The bug: if \ is at index 0, it doesn't escape the next quote.
        let haystack = b"\\\"foo\"";
        // indices:
        // 0: \
        // 1: "  (escaped)
        // 2: f
        // 3: o
        // 4: o
        // 5: "  (structural)
        
        let quotes: Vec<usize> = JsonQuotes::new(haystack).collect();
        assert_eq!(quotes, vec![5]);
        
        let haystack = b"{\"a\": \"\\\"b\"}";
        // 0: {
        // 1: " (start key)
        // 2: a
        // 3: " (end key)
        // 4: :
        // 5:  
        // 6: " (start val)
        // 7: \
        // 8: " (escaped quote)
        // 9: b
        // 10: " (end val)
        // 11: }
        
        let quotes: Vec<usize> = JsonQuotes::new(haystack).collect();
        assert_eq!(quotes, vec![1, 3, 6, 10]);
    }

    #[test]
    fn test_escaped_quote_at_start() {
        // More direct test for the index-0 bug
        let haystack = b"\\\""; // \ at 0, " at 1
        let quotes: Vec<usize> = JsonQuotes::new(haystack).collect();
        assert!(quotes.is_empty(), "Quote at index 1 should be escaped by \\ at index 0");
    }

    #[test]
    fn test_double_escape() {
        let haystack = b"\\\\\""; // \ at 0, \ at 1, " at 2
        // \ at 1 is escaped by \ at 0.
        // So " at 2 is NOT escaped.
        let quotes: Vec<usize> = JsonQuotes::new(haystack).collect();
        assert_eq!(quotes, vec![2]);
    }

    #[test]
    fn test_range_iter() {
        let haystack = b"{\"foo\": \"bar\"}";
        // 0: {
        // 1: "
        // 2: f
        // 3: o
        // 4: o
        // 5: "
        // 6: :
        // 7: 
        // 8: "
        // 9: b
        // 10: a
        // 11: r
        // 12: "
        // 13: }
        let ranges: Vec<(usize, usize)> = jsonquotes_range_iter(haystack).collect();
        assert_eq!(ranges, vec![(1, 6), (8, 13)]);
        
        assert_eq!(&haystack[ranges[0].0..ranges[0].1], b"\"foo\"");
        assert_eq!(&haystack[ranges[1].0..ranges[1].1], b"\"bar\"");
    }
}
