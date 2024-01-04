use crate::jsonquotes::jsonquotes_range_iter;
use anyhow::{bail, Error, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use clap::{Parser, ValueEnum};
use grep_cli::{self, stdout};
use std::fs::File;
use std::io::{self, BufReader, IsTerminal, Write};
use std::path::Path;
use std::process::exit;
use termcolor::ColorChoice;

pub mod build;
pub mod fstsed;
pub mod jsonquotes;

const BUFFERSIZE: usize = 64 * 1024;

// via https://github.com/sstadick/hck/blob/master/src/main.rs#L90
/// Check if err is a broken pipe.
#[inline]
fn is_broken_pipe(err: &Error) -> bool {
    if let Some(io_err) = err.root_cause().downcast_ref::<io::Error>() {
        if io_err.kind() == io::ErrorKind::BrokenPipe {
            return true;
        }
    }
    false
}

// via https://github.com/sstadick/crabz/blob/ce0d69efe0628c56b1fb7a1de46798b95eef90aa/src/main.rs#L62
/// Get a buffered input reader from stdin or a file
fn get_input(path: Option<Utf8PathBuf>) -> Result<Box<dyn BufReadExt + Send + 'static>> {
    let reader: Box<dyn BufReadExt + Send + 'static> = match path {
        Some(path) => {
            if path.as_os_str() == "-" {
                Box::new(BufReader::with_capacity(BUFFERSIZE, io::stdin()))
            } else {
                Box::new(BufReader::with_capacity(BUFFERSIZE, File::open(path)?))
            }
        }
        None => Box::new(BufReader::with_capacity(BUFFERSIZE, io::stdin())),
    };
    Ok(reader)
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Show only nonempty parts of lines that match
    #[clap(short, long)]
    only_matching: bool,

    /// Use markers to highlight the matching strings
    #[clap(short = 'C', long, value_enum, default_value_t = ArgsColorChoice::Auto)]
    color: ArgsColorChoice,

    /// Specify fst db to use in search or build modes
    #[clap(short = 'f', value_name = "FST", value_hint = clap::ValueHint::FilePath)]
    fst: Utf8PathBuf,

    /// Build mode. Build a fst from json data instead of querying one. Specify output path with
    /// the -f --fst parameter. Only first file input parameter or stdin is used to make
    /// the fst
    #[clap(long)]
    build: bool,

    /// When building a fst, extract the given json field to use as the key in the fst database
    #[clap(short = 'k', long, value_name = "KEY", default_value = "key")]
    key: Option<String>,

    /// Specify the format of the fstsed match decoration. Field names are enclosed in {},
    /// for example "{field1} any fixed string {field2} & {field3}"
    #[clap(short, long)]
    template: Option<String>,

    /// Json search mode. Fstsed will treat input as json, searching only inside quoted json strings.
    /// All strings are deserialized/decoded before json before searching, and all template
    /// decorations are properly json-encoded in the output for subsequent processing
    #[clap(short, long)]
    json: bool,

    /// Input file(s) to process (either to search or to use to build the fst). Leave empty or
    /// use "-" to read from stdin
    #[clap(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    input: Vec<Utf8PathBuf>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
enum ArgsColorChoice {
    Always,
    Never,
    Auto,
}

fn main() -> Result<()> {
    let mut args = Args::parse();

    // if no files specified, add stdin
    if args.input.is_empty() {
        args.input.push(Utf8PathBuf::from("-"));
    }

    // determine appropriate colormode. auto simply
    // tests if stdout is a tty (if so, then yes color)
    // or otherwise don't color if it's to a file or another pipe
    let colormode = match args.color {
        ArgsColorChoice::Auto => {
            if std::io::stdout().is_terminal() {
                ColorChoice::Always
            } else {
                ColorChoice::Never
            }
        }
        ArgsColorChoice::Always => ColorChoice::Always,
        ArgsColorChoice::Never => ColorChoice::Never,
    };

    // invoke the command!
    if let Err(e) = if args.build {
        run_build(args)
    } else if args.only_matching {
        run_onlymatching(args, colormode)
    } else if args.json {
        runjson(args, colormode)
    } else {
        run(args, colormode)
    } {
        // safely ignore broken pipes, e.g. head
        if is_broken_pipe(&e) {
            exit(0);
        }
        return Err(e);
    }
    Ok(())
}

#[inline]
fn run_build(args: Args) -> Result<()> {
    // ensure the fst path does not already exist. don't want to overwrite
    if Path::new(&args.fst).exists() {
        bail!("fst path {} already exists. Please specify an alternate path or rename/delete existing fst.", &args.fst);
    }
    // currently, just grab the first input item
    let reader = get_input(args.input.first().cloned()).expect("need some input");
    build::build_fstsed(reader, &args.key.unwrap(), &args.fst)
}

// Generic processing function that we use in all modes to search the given
// input wth the given fstsed db and write to the given output
#[inline]
fn process_line<W>(input: &[u8], fsed: &fstsed::FstSed, out: &mut W) -> Result<(), Error>
where
    W: Write + Send + 'static,
{
    let mut _lastpos: usize = 0;
    // process each line
    for m in fsed.find_iter(input) {
        // print gap from last match to current match
        out.write_all(&input[_lastpos..m])?;
        // print rendered match
        out.write_all(fsed.get_match().render().as_bytes())?;
        // advance the position past our match length
        _lastpos = m + fsed.get_match_len();
    }
    // print remainder
    out.write_all(&input[_lastpos..])?;

    Ok(())
}

// Basic mode
#[inline]
fn run(args: Args, colormode: ColorChoice) -> Result<(), Error> {
    let mut out = stdout(colormode);
    let fsed = fstsed::FstSed::new(args.fst, args.template, colormode);

    for path in args.input {
        let mut reader = get_input(Some(path))?;
        reader.for_byte_line_with_terminator(|line| {
            // TODO: i cant figure out how to transform the std::io::error into anyhow
            process_line(line, &fsed, &mut out);
            Ok(true)
        })?;
    }
    out.flush()?;
    Ok(())
}

// Print just the search matches rather than the entire line
#[inline]
fn run_onlymatching(args: Args, colormode: ColorChoice) -> Result<()> {
    let mut out = stdout(colormode);
    let fsed = fstsed::FstSed::new(args.fst, args.template, colormode);

    for path in args.input {
        let mut reader = get_input(Some(path))?;
        reader.for_byte_line_with_terminator(|line| {
            for _ in fsed.find_iter(line) {
                // just print rendered match and a new line
                out.write_all(fsed.get_match().render().as_bytes())?;
                out.write_all(b"\n")?;
            }
            Ok(true)
        })?;
    }
    out.flush()?;
    Ok(())
}

// Json search mode. Use the jsonquotes utility in this crate to find and deserialize just the
// json strings in the input. Also ensure all formatted output is properly json encoded.
#[inline]
fn runjson(args: Args, _: ColorChoice) -> Result<(), Error> {
    // cant colorize text inside of json strings
    let mut out = stdout(ColorChoice::Never);
    let fsed = fstsed::FstSed::new(args.fst, args.template, ColorChoice::Never);

    // temp buffer for holding processed string before re-serializing
    let mut buf = Vec::with_capacity(BUFFERSIZE);

    for path in args.input {
        let mut reader = get_input(Some(path))?;
        reader.for_byte_line_with_terminator(|line| {
            let mut lastpos: usize = 0;
            for (start, end) in jsonquotes_range_iter(line) {
                // print from last spot to new start
                out.write_all(&line[lastpos..start])?;
                // deserialize string and process result
                // note: we are allocating a new string every time
                match serde_json::from_slice::<String>(&line[start..end]) {
                    Ok(s) => {
                        buf.clear();
                        // reuse vec buf to collect the processed line
                        process_line(s.as_bytes(), &fsed, &mut buf);
                        // serialize new json string directly to the output
                        serde_json::to_writer(&mut out, std::str::from_utf8(&buf).unwrap())?;
                    }
                    // if error deserializing, just print the original content and move on
                    // we're not here to enforce json formats
                    _ => out.write_all(&line[start..end])?,
                };
                // advance position
                lastpos = end;
            }
            // print remainder
            out.write_all(&line[lastpos..])?;
            Ok(true)
        })?;
    }
    out.flush()?;
    Ok(())
}
