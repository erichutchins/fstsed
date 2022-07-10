use anyhow::{Error, Result};
use camino::Utf8PathBuf;
use clap::{ArgEnum, Parser};
use fst::raw::{Fst, Output};
use grep_cli::{self, stdout};
use memmap2::Mmap;
use regex::bytes::Regex;
use ripline::{
    line_buffer::{LineBufferBuilder, LineBufferReader},
    lines::LineIter,
    LineTerminator,
};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::process::exit;
use termcolor::ColorChoice;

const BUFFERSIZE: usize = 64 * 1024;
const SENTINEL: u8 = 0;

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
fn get_input(path: Option<Utf8PathBuf>) -> Result<Box<dyn Read + Send + 'static>> {
    let reader: Box<dyn Read + Send + 'static> = match path {
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

// from https://github.com/BurntSushi/fst/blob/master/fst-bin/src/util.rs
#[inline]
unsafe fn mmap_fst(path: Utf8PathBuf) -> Result<Fst<Mmap>, Error> {
    let mmap = Mmap::map(&File::open(path)?)?;
    let fst = Fst::new(mmap)?;
    Ok(fst)
}

// adapted from https://github.com/BurntSushi/fst/pull/104/files
#[inline]
fn find_longest_prefix_sentinel<D: AsRef<[u8]>>(
    fst: &Fst<D>,
    value: &[u8],
) -> Option<(usize, String)> {
    let mut node = fst.root();
    let mut out = Output::zero();
    let mut last_match = None;
    for (i, &b) in value.iter().enumerate() {
        if let Some(trans_index) = node.find_input(b) {
            let t = node.transition(trans_index);
            node = fst.node(t.addr);
            out = out.cat(t.out);

            if let Some(sentinel_index) = node.find_input(SENTINEL) {
                let sentinel = node.transition(sentinel_index);
                let mut snode = fst.node(sentinel.addr);
                let mut bytes = vec![];
                while !snode.is_final() {
                    if let Some(t) = snode.transitions().next() {
                        // after the sentinel, we should not have any more
                        // branching in the fst, so we just grab the first transition
                        bytes.push(t.inp);
                        snode = fst.node(t.addr);
                    } else {
                        // somehow ran out of nodes!
                        break;
                    }
                }
                last_match = Some((i + 1, unsafe { String::from_utf8_unchecked(bytes) }));
            }
        } else {
            return last_match;
        }
    }
    last_match
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
    let fst = unsafe { mmap_fst(args.fst).unwrap() };

    let mut out = stdout(ColorChoice::Auto);
    let re = Regex::new(r"\W").unwrap();

    for path in args.input {
        let reader = get_input(Some(path))?;
        let terminator = LineTerminator::byte(b'\n');
        let mut line_buffer = LineBufferBuilder::new().build();
        let mut lb_reader = LineBufferReader::new(reader, &mut line_buffer);

        // line reader
        while lb_reader.fill()? {
            let lines = LineIter::new(terminator.as_byte(), lb_reader.buffer());
            for mut input in lines {
                // process each line
                while !input.is_empty() {
                    match find_longest_prefix_sentinel(&fst, input) {
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
                        Some((len, value)) => {
                            // we have a match! len is the size of the input buffer that matched
                            // a key in our fst. value is the corresponding remainder of the key0value
                            // concatenated string in our fst
                            if len != 0 && (len == input.len() || re.is_match(&input[len..len + 1]))
                            {
                                out.write_all(b"<")?;
                                out.write_all(&input[..len])?;
                                out.write_all(b"|")?;
                                out.write_all(value.as_bytes())?;
                                out.write_all(b">")?;
                            } else {
                                // our match landed in the middle of a word
                                out.write_all(&input[..len])?;
                            }
                            // advance the line buffer
                            input = &input[len..];
                        }
                    }; // match
                } // while input
            } // for each line
            lb_reader.consume_all();
        } // while lbreader
    } // for each path

    out.flush()?;
    Ok(())
}

#[inline]
fn run_onlymatching(args: Args, colormode: ColorChoice) -> Result<()> {
    let fst = Fst::from_iter_map(vec![
        ("a0one", 1),
        ("ab0two", 2),
        ("abc0three", 3),
        ("abc0uni", 6),
        ("bc0four", 4),
        ("hello world0multi-word test", 7),
        ("uvwxyz0five", 5),
    ])
    .unwrap();

    let mut out = stdout(ColorChoice::Auto);
    let re = Regex::new(r"\W").unwrap();

    let reader = get_input(None)?;
    let terminator = LineTerminator::byte(b'\n');
    let mut line_buffer = LineBufferBuilder::new().build();
    let mut lb_reader = LineBufferReader::new(reader, &mut line_buffer);

    // line reader
    while lb_reader.fill()? {
        let lines = LineIter::new(terminator.as_byte(), lb_reader.buffer());
        for mut input in lines {
            // process each line
            while !input.is_empty() {
                match find_longest_prefix_sentinel(&fst, input) {
                    Some((len, value)) => {
                        // we have a match! len is the size of the input buffer that matched
                        // a key in our fst. value is the corresponding remainder of the key0value
                        // concatenated string in our fst
                        if len != 0 && (len == input.len() || re.is_match(&input[len..len + 1])) {
                            out.write_all(b"<")?;
                            out.write_all(&input[..len])?;
                            out.write_all(b"|")?;
                            out.write_all(value.as_bytes())?;
                            out.write_all(b">")?;
                            // and a newline
                            out.write_all(&[b'\n'])?;
                        }
                        // advance the line buffer
                        input = &input[len..];
                    }
                    None => {
                        // no match, so advance the line buffer to the next
                        // word boundary and search again
                        if let Some(nextword) = re.find(input) {
                            input = &input[nextword.start() + 1..];
                            continue;
                        } else {
                            // no more words, we're done
                            break;
                        }
                    }
                }; // match
            } // while
        } // for
        lb_reader.consume_all();
    }

    out.flush()?;
    Ok(())
}
