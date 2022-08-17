use itermore::Itermore;
use memchr::memchr2_iter;
use memchr::Memchr2;

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

#[inline]
pub fn jsonquotes_range_iter<'a>(
    haystack: &'a [u8],
) -> Box<dyn Iterator<Item = (usize, usize)> + 'a> {
    // box magic from https://stackoverflow.com/a/31904898
    Box::new(
        JsonQuotes::new(haystack)
            .into_iter()
            .array_chunks::<2>()
            .map(move |[a, b]| (a, b + 1)),
    )
}
