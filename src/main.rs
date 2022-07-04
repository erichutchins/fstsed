use anyhow::Result;
use camino::Utf8PathBuf;
use fst::raw::{Fst, Output};
use grep_cli::{self, stdout};
use regex::bytes::Regex;
use ripline::{
    line_buffer::{LineBufferBuilder, LineBufferReader},
    lines::LineIter,
    LineTerminator,
};
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use termcolor::ColorChoice;

const BUFFERSIZE: usize = 64 * 1024;
const SENTINEL: u8 = b"0"[0];

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
                last_match = Some((i + 1, String::from_utf8(bytes).unwrap()));
            }
        } else {
            return last_match;
        }
    }
    last_match
}

fn main() -> Result<()> {
    run_onlymatching()
}

#[inline]
fn run() -> Result<()> {
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
                        } else {
                            // our match landed in the middle of a word
                            out.write_all(&input[..len])?;
                        }
                        // advance the line buffer
                        input = &input[len..];
                    }
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
                }; // match
            } // while
        } // for
        lb_reader.consume_all();
    }

    out.flush()?;
    Ok(())
}

#[inline]
fn run_onlymatching() -> Result<()> {
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
