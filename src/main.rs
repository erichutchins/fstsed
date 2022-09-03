use crate::jsonquotes::jsonquotes_range_iter;
use anyhow::{Error, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use clap::{ArgEnum, Parser};
use grep_cli::{self, stdout};
use lazy_static::lazy_static;
use regex::bytes::Regex;
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::process::exit;
use termcolor::ColorChoice;

pub mod fstsed;
pub mod jsonquotes;

const BUFFERSIZE: usize = 64 * 1024;

lazy_static! {
    static ref RE_NONWORD: Regex = Regex::new(r"(?i-u)\W").unwrap();
}

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

// via https://github.com/sstadick/crabz/blob/main/src/main.rs#L82
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
    #[clap(short = 'C', long, arg_enum, default_value_t = ArgsColorChoice::Auto)]
    color: ArgsColorChoice,

    /// Specify fst db to use
    #[clap(short = 'f', value_name = "FST", value_hint = clap::ValueHint::FilePath)]
    fst: Utf8PathBuf,

    /// Specify the format of the fstsed match decoration. Field names are enclosed in {},
    /// for example "{field1} any fixed string {field2} & {field3}"
    #[clap(short, long)]
    template: Option<String>,

    /// Specify json input. Fstsed will unescape json strings before searching and ensure
    /// output is json-safe
    #[clap(short, long)]
    json: bool,

    /// Input file(s) to process. Leave empty or use "-" to read from stdin
    #[clap(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    input: Vec<Utf8PathBuf>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ArgEnum)]
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
            if grep_cli::is_tty_stdout() {
                ColorChoice::Always
            } else {
                ColorChoice::Never
            }
        }
        ArgsColorChoice::Always => ColorChoice::Always,
        ArgsColorChoice::Never => ColorChoice::Never,
    };

    // invoke the command!
    if let Err(e) = if args.only_matching {
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
fn runjson(args: Args, colormode: ColorChoice) -> Result<(), Error> {
    let mut out = stdout(colormode);

    let mut fsed = fstsed::FstSed::new(args.fst, args.template, colormode);

    for path in args.input {
        let reader = get_input(Some(path))?;
        reader.for_byte_line_with_terminator(|line| {
            let mut lastpos: usize = 0;
            for (start, end) in jsonquotes_range_iter(line) {
                // print from last spot to new start
                out.write_all(&line[lastpos..start])?;
                // search the quoted string
                process_line(&line[start..end], &mut fsed, &mut out);
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

#[inline]
fn process_line<W>(input: &[u8], fsed: &mut fstsed::FstSed, out: &mut W) -> Result<(), Error>
where
    W: Write + Send + 'static,
{
    let mut _lastpos: usize = 0;
    // process each line
    for m in fsed.find_iter(input) {
        // print gap from last match to current match
        out.write_all(&input[_lastpos..=m.start()])?;

        // print rendered match
        out.write_all(fsed.get_match().render().as_bytes())?;
        // have to add one! the m.start is unfortunely the word boundary pos
        // not the actual start of the matching text
        _lastpos = m.start() + 1 + fsed.get_match_len();
    }
    // print remainder
    out.write_all(&input[_lastpos..])?;

    Ok(())
}

#[inline]
fn run(args: Args, colormode: ColorChoice) -> Result<(), Error> {
    let mut out = stdout(colormode);

    let mut fsed = fstsed::FstSed::new(args.fst, args.template, colormode);
    for path in args.input {
        let reader = get_input(Some(path))?;
        reader.for_byte_line_with_terminator(|line| {
            process_line(line, &mut fsed, &mut out);
            Ok(true)
        })?;
    }
    out.flush()?;
    Ok(())
}

#[inline]
fn run_onlymatching(args: Args, colormode: ColorChoice) -> Result<()> {
    let mut out = stdout(colormode);

    let mut fsed = fstsed::FstSed::new(args.fst, args.template, colormode);
    for path in args.input {
        let reader = get_input(Some(path))?;
        reader.for_byte_line_with_terminator(|line| {
            for m in fsed.find_iter(line) {
                // print rendered match and a new line
                out.write_all(fsed.get_match().render().as_bytes())?;
                out.write_all(b"\n")?;
            }
            Ok(true)
        })?;
    }
    out.flush()?;
    Ok(())
}
