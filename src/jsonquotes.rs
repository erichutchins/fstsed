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
    lastescape: usize,
}

impl<'a> JsonQuotes<'a> {
    pub fn new(haystack: &'a [u8]) -> Self {
        Self {
            haystack,
            iter: memchr2_iter(b'"', b'\\', haystack),
            lastescape: 0,
        }
    }
}

impl<'a> Iterator for JsonQuotes<'a> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        for index in self.iter.by_ref() {
            if self.haystack[index] == b'"' {
                if self.lastescape > 0 && self.lastescape == index - 1 {
                    // a true escaped quote! reset the counter and continue
                    // to next memchr2 match
                    self.lastescape = 0;
                    continue;
                }
                // we have a structural quote. reset escape counter
                // and return the index position
                self.lastescape = 0;
                return Some(index);
            } else {
                // we have a \
                if self.lastescape == index - 1 {
                    // we just saw an escape and now we have another one
                    // this is a \\ double escape, so we turn off
                    self.lastescape = 0;
                } else {
                    self.lastescape = index;
                }
            }
        }
        // exhausted the haystack, we're done
        None
    }
}

/// Isolate just the ranges of strings in json to avoid deserializing the entire structure. Uses
/// memchr2 to find all doublequotes and backslashes simulktaneously and tracks state to determine
/// when the backslashes escape the quotes.
///
/// This function returns an iterator of (start, end) tuples of the string ranges. *Note* the
/// indices include the quotation marks themselves!
#[inline]
pub fn jsonquotes_range_iter<'a>(
    haystack: &'a [u8],
) -> Box<dyn Iterator<Item = (usize, usize)> + 'a> {
    // box magic from https://stackoverflow.com/a/31904898
    Box::new(
        // Rather than have JsonQuotes bother with knowing if a quote is an open or close,
        // the indices come to us in a flat series and we just iterate in chunks of
        // two giving us each start, stop index. We add 1 so when this tuple is used to
        // retrieve the str, both open and close quotes are themselves included
        IterArrayChunks::array_chunks::<2>(JsonQuotes::new(haystack)).map(move |[a, b]| (a, b + 1)),
    )
}
