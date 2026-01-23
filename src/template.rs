//! Template rendering utilities.
//!
//! This module is a zero-allocation rendering engine that writes directly to a
//! `std::fmt::Formatter`. The logic is adapted from the `microtemplate` crate
//! (https://github.com/Legend-of-iPhoenix/microtemplate).

use std::fmt;

/// A trait for providing values to templates.
pub trait Context {
    /// Returns the value for a given field name.
    fn get_field(&self, field_name: &str) -> &str;
}

/// Renders a template string using the provided context, writing to a formatter.
///
/// This implementation avoids intermediate string allocations by writing slices
/// directly to the output.
pub fn render_fmt<C: Context>(input: &str, context: C, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let input_bytes = input.as_bytes();
    let mut iter = input_bytes.iter().copied().enumerate();
    let mut last_index = 0;

    while let Some((index, c)) = iter.next() {
        if c == b'{' {
            // Write text before the opening brace
            // SAFETY: We iterate over valid UTF-8, so slicing between known chars is safe.
            let pre_text = unsafe {
                std::str::from_utf8_unchecked(input_bytes.get_unchecked(last_index..index))
            };
            f.write_str(pre_text)?;

            for (substitution_index, c) in iter.by_ref() {
                if c == b'}' {
                    let field_name = unsafe {
                        // +1 skips the opening {
                        std::str::from_utf8_unchecked(
                            input_bytes.get_unchecked(index + 1..substitution_index),
                        )
                    };

                    f.write_str(context.get_field(field_name))?;

                    // +1 skips the closing }
                    last_index = substitution_index + 1;
                    break;
                }
            }
        }
    }

    // Write remaining text
    let remaining = unsafe {
        std::str::from_utf8_unchecked(input_bytes.get_unchecked(last_index..input.len()))
    };
    f.write_str(remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct TestContext;
    impl Context for TestContext {
        fn get_field(&self, name: &str) -> &str {
            match name {
                "foo" => "bar",
                "hello" => "world",
                _ => "",
            }
        }
    }

    struct Renderer<'a> {
        template: &'a str,
        context: TestContext,
    }

    impl fmt::Display for Renderer<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            render_fmt(self.template, self.context, f)
        }
    }

    #[test]
    fn test_render_fmt_basic() {
        let renderer = Renderer {
            template: "Start {foo} end",
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "Start bar end");
    }

    #[test]
    fn test_render_fmt_multiple() {
        let renderer = Renderer {
            template: "{hello} {foo}!",
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "world bar!");
    }

    #[test]
    fn test_render_fmt_no_subs() {
        let renderer = Renderer {
            template: "Just text",
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "Just text");
    }

    #[test]
    fn test_render_fmt_missing() {
        let renderer = Renderer {
            template: "Missing {baz}",
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "Missing ");
    }
}
