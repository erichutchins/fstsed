use anyhow::{Error, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use clap::{ArgEnum, Parser};
use grep_cli::{self, stdout};
use regex::bytes::Regex;
use std::fs::File;
use std::io::{self, BufReader, Write};
use std::process::exit;
use termcolor::ColorChoice;

pub mod fstsed;

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
fn run(args: Args, colormode: ColorChoice) -> Result<()> {
    let mut out = stdout(ColorChoice::Auto);
    let re = Regex::new(r"\W").unwrap();

    // pub fn new(fstpath: Utf8PathBuf, user_template: Option<String>, color: ColorChoice) -> Self {
    let mut fsed = fstsed::FstSed::new(args.fst, args.template, colormode);

    for path in args.input {
        let reader = get_input(Some(path))?;
        for byteline in reader.byte_lines() {
            let linevec = byteline.unwrap();
            let mut input = linevec.as_slice();
            // process each line
            while !input.is_empty() {
                match fsed.longest_match(input) {
                    None => {
                        // no match, so advance the line buffer to the next
                        // word boundary and search again
                        if let Some(nextword) = re.find(input) {
                            out.write_all(&input[..nextword.start() + 1])?;
                            input = &input[nextword.start() + 1..];
                            continue;
                        } else {
                            // no more words, so just print remainder of the line
                            out.write_all(input)?;
                            break;
                        }
                    }
                    Some(len) => {
                        // we have a match! len is the size of the input buffer that matched
                        out.write_all(fsed.render_match().as_bytes())?;
                        // advance the line buffer
                        input = &input[len..];
                    }
                }; // match
            } // while input
            out.write_all(b"\n")?;
        } // for each line
    } // for each path

    out.flush()?;
    Ok(())
}

#[inline]
fn run_onlymatching(args: Args, colormode: ColorChoice) -> Result<()> {
    let mut out = stdout(ColorChoice::Auto);
    let re = Regex::new(r"\W").unwrap();

    let mut fsed = fstsed::FstSed::new(args.fst, args.template, colormode);

    for path in args.input {
        let reader = get_input(Some(path))?;
        for byteline in reader.byte_lines() {
            let linevec = byteline.unwrap();
            let mut input = linevec.as_slice();
            // process each line
            while !input.is_empty() {
                match fsed.longest_match(input) {
                    None => {
                        // no match, so advance the line buffer to the next
                        // word boundary and search again
                        if let Some(nextword) = re.find(input) {
                            input = &input[nextword.start() + 1..];
                            continue;
                        } else {
                            // no more words, so just print remainder of the line
                            out.write_all(input)?;
                            break;
                        }
                    }
                    Some(len) => {
                        // we have a match! len is the size of the input buffer that matched
                        out.write_all(fsed.render_match().as_bytes())?;
                        // advance the line buffer
                        input = &input[len..];
                    }
                }; // match
            } // while line
            out.write_all(b"\n")?;

        } // for line
    } // for path

    out.flush()?;
    Ok(())
}
