//! Template rendering utilities.
//!
//! This module is a zero-allocation rendering engine that writes directly to a
//! `std::fmt::Formatter`. The logic is adapted from the `microtemplate` crate
//! (<https://github.com/Legend-of-iPhoenix/microtemplate>).

use std::fmt;

/// A trait for providing values to templates.
pub trait Context {
    /// Returns the value for a given field name.
    fn get_field(&self, field_name: &str) -> &str;
}

/// A part of a template, either literal text or a variable substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePart {
    /// Fixed text.
    Literal(String),
    /// A variable to be substituted (e.g., "key", "value", or a JSON pointer).
    Variable(String),
}

/// A pre-parsed match template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Template {
    /// The default template: `<{key}|{value}>`.
    Standard,
    /// Just the key: `{key}`.
    KeyOnly,
    /// Just the value: `{value}`.
    ValueOnly,
    /// A custom user-defined template.
    Custom(Vec<TemplatePart>),
}

impl Template {
    /// The string representation of the default template.
    pub const DEFAULT: &'static str = "<{key}|{value}>";

    /// Parses a template string into a structured `Template`.
    pub fn parse(input: &str) -> Self {
        match input {
            Self::DEFAULT => Self::Standard,
            "{key}" => Self::KeyOnly,
            "{value}" => Self::ValueOnly,
            _ => {
                let mut parts = Vec::new();
                // SAFETY: Byte-level parsing is safe here because `{` (0x7B) and `}` (0x7D)
                // are ASCII characters that can never appear as UTF-8 continuation bytes.
                // UTF-8 continuation bytes always have the high bit pattern 10xxxxxx (0x80-0xBF).
                let input_bytes = input.as_bytes();
                let mut last_index = 0;
                let mut i = 0;

                while i < input_bytes.len() {
                    if input_bytes[i] == b'{' {
                        // Push preceding literal if any
                        if i > last_index {
                            parts.push(TemplatePart::Literal(input[last_index..i].to_string()));
                        }

                        // Find closing brace
                        let start = i + 1;
                        if let Some(end) = input_bytes[start..].iter().position(|&b| b == b'}') {
                            let end = start + end;
                            parts.push(TemplatePart::Variable(input[start..end].to_string()));
                            last_index = end + 1;
                            i = end;
                        } else {
                            // No closing brace found, treat rest as literal
                            break;
                        }
                    }
                    i += 1;
                }

                if last_index < input.len() {
                    parts.push(TemplatePart::Literal(input[last_index..].to_string()));
                }

                Self::Custom(parts)
            }
        }
    }

    /// Renders the template using the provided context.
    pub fn render<C: Context>(&self, context: &C, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Standard => {
                f.write_str("<")?;
                f.write_str(context.get_field("key"))?;
                f.write_str("|")?;
                f.write_str(context.get_field("value"))?;
                f.write_str(">")
            }
            Self::KeyOnly => f.write_str(context.get_field("key")),
            Self::ValueOnly => f.write_str(context.get_field("value")),
            Self::Custom(parts) => {
                for part in parts {
                    match part {
                        TemplatePart::Literal(s) => f.write_str(s)?,
                        TemplatePart::Variable(v) => f.write_str(context.get_field(v))?,
                    }
                }
                Ok(())
            }
        }
    }
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

    struct Renderer {
        template: Template,
        context: TestContext,
    }

    impl fmt::Display for Renderer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.template.render(&self.context, f)
        }
    }

    #[test]
    fn test_render_fmt_basic() {
        let renderer = Renderer {
            template: Template::parse("Start {foo} end"),
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "Start bar end");
    }

    #[test]
    fn test_render_fmt_multiple() {
        let renderer = Renderer {
            template: Template::parse("{hello} {foo}!"),
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "world bar!");
    }

    #[test]
    fn test_render_fmt_no_subs() {
        let renderer = Renderer {
            template: Template::parse("Just text"),
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "Just text");
    }

    #[test]
    fn test_render_fmt_missing() {
        let renderer = Renderer {
            template: Template::parse("Missing {baz}"),
            context: TestContext,
        };
        assert_eq!(renderer.to_string(), "Missing ");
    }

    #[test]
    fn test_template_standard() {
        let renderer = Renderer {
            template: Template::parse(Template::DEFAULT),
            context: TestContext,
        };
        // TestContext returns "" for key/value
        assert_eq!(renderer.to_string(), "<|>");
    }
}
