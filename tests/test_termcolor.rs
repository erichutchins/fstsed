use std::io::Write;
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

fn main() {
    let choice = ColorChoice::Auto;
    let writer = BufferWriter::stdout(choice);
    let mut buf = writer.buffer();

    let mut spec = ColorSpec::new();
    spec.set_fg(Some(Color::Red));

    buf.set_color(&spec).unwrap();
    write!(&mut buf, "HIGHLIGHTED").unwrap();
    buf.reset().unwrap();

    writer.print(&buf).unwrap();
}
