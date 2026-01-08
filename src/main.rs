use anyhow::{bail, Result};
use camino::Utf8PathBuf;
use clap::Parser;
use fstsed::cli::{Args, ArgsColorChoice};
use fstsed::fstsed::{FstCache, FstSed};
use fstsed::jsonquotes::jsonquotes_range_iter;
use fstsed::BUFFERSIZE;
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, Sink, SinkMatch};
use ignore::WalkBuilder;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use termcolor::{Buffer, BufferWriter, ColorChoice};

fn main() -> Result<()> {
    let mut args = Args::parse();

    // if no files specified, default to stdin
    if args.input.is_empty() {
        args.input.push(Utf8PathBuf::from("-"));
    }

    if args.build {
        return run_build(args);
    }

    // determine appropriate colormode
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

    // Initialize FstSed and wrap in Arc for sharing across threads
    let fsed = Arc::new(FstSed::new(args.fst, args.template, colormode));

    // For stdout output
    let writer = Arc::new(BufferWriter::stdout(colormode));

    // Differentiate between stdin and file paths
    let has_stdin = args.input.iter().any(|p| p == "-");

    if has_stdin {
        // Single threaded stdin processing
        let mut buf = writer.buffer();
        let mut cache = FstCache::new();
        // Match start of every line
        let matcher = RegexMatcher::new(r"(?m)^").unwrap();

        let sink = FstSedSink {
            fsed: &fsed,
            cache: &mut cache,
            buf: &mut buf,
            only_matching: args.only_matching,
            json: args.json,
            temp_buf: Vec::with_capacity(BUFFERSIZE),
        };

        // Search stdin
        Searcher::new().search_reader(&matcher, io::stdin(), sink)?;
        writer.print(&buf)?;
    } else {
        // Parallel directory walking
        let mut builder = WalkBuilder::new(&args.input[0]);
        for input in &args.input[1..] {
            builder.add(input);
        }

        if let Some(threads) = args.threads {
            builder.threads(threads);
        }

        if args.no_ignore {
            builder
                .ignore(false)
                .git_global(false)
                .git_ignore(false)
                .git_exclude(false);
        }

        if args.hidden {
            builder.hidden(false);
        }

        builder.build_parallel().run(|| {
            let fsed = fsed.clone();
            let writer = writer.clone();
            let mut buf = writer.buffer();
            let mut cache = FstCache::new();
            // Match start of every line
            let matcher = RegexMatcher::new(r"(?m)^").unwrap();

            Box::new(move |result| {
                let entry = match result {
                    Ok(entry) => entry,
                    Err(err) => {
                        eprintln!("{}", err);
                        return ignore::WalkState::Continue;
                    }
                };

                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }

                let sink = FstSedSink {
                    fsed: &fsed,
                    cache: &mut cache,
                    buf: &mut buf,
                    only_matching: args.only_matching,
                    json: args.json,
                    temp_buf: Vec::with_capacity(BUFFERSIZE),
                };

                let mut searcher = Searcher::new();
                if let Err(err) = searcher.search_path(&matcher, entry.path(), sink) {
                    eprintln!("{}: {}", entry.path().display(), err);
                }

                // Flush thread-local buffer to stdout (atomically)
                let _ = writer.print(&buf);
                buf.clear();

                ignore::WalkState::Continue
            })
        });
    }

    Ok(())
}

#[inline]
fn run_build(args: Args) -> Result<()> {
    // ensure the fst path does not already exist
    if Path::new(&args.fst).exists() {
        bail!(
            "fst path {} already exists. Please specify an alternate path or rename/delete existing fst.",
            &args.fst
        );
    }
    // handle stdin vs file for build
    let input = args.input.first().cloned().unwrap();
    let reader: Box<dyn bstr::io::BufReadExt> = if input == "-" {
        Box::new(io::BufReader::with_capacity(BUFFERSIZE, io::stdin()))
    } else {
        Box::new(io::BufReader::with_capacity(
            BUFFERSIZE,
            std::fs::File::open(input)?,
        ))
    };

    fstsed::build::build_fstsed(reader, &args.key.unwrap(), &args.fst, args.sorted)
}

/// FstSedSink acts as the bridge between grep-searcher and fstsed.
/// It receives lines from the searcher, processes them with fstsed,
/// and writes the result to the provided buffer.
struct FstSedSink<'a, 'b> {
    fsed: &'a FstSed,
    cache: &'b mut FstCache,
    buf: &'b mut Buffer,
    only_matching: bool,
    json: bool,
    // scratch buffer for JSON string handling
    temp_buf: Vec<u8>,
}

impl<'a, 'b> Sink for FstSedSink<'a, 'b> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let line = mat.bytes();
        if self.json {
            self.process_json(line)
        } else {
            self.process_text(line)
        }
    }
}

impl<'a, 'b> FstSedSink<'a, 'b> {
    #[inline]
    fn process_text(&mut self, input: &[u8]) -> Result<bool, io::Error> {
        let mut lastpos: usize = 0;
        let mut matcher = self.fsed.find_iter(input, self.cache);

        while let Some(m) = matcher.next() {
            if !self.only_matching {
                self.buf.write_all(&input[lastpos..m])?;
            }
            // print rendered match
            self.buf
                .write_all(matcher.get_match().render().as_bytes())?;
            if self.only_matching {
                self.buf.write_all(b"\n")?;
            }
            lastpos = m + matcher.get_match_len();
        }

        if !self.only_matching {
            // print remainder
            self.buf.write_all(&input[lastpos..])?;
        }

        Ok(true)
    }

    #[inline]
    fn process_json(&mut self, line: &[u8]) -> Result<bool, io::Error> {
        let mut lastpos: usize = 0;
        for (start, end, has_esc) in jsonquotes_range_iter(line) {
            // print from last spot to new start
            if !self.only_matching {
                self.buf.write_all(&line[lastpos..start])?;
            }

            // CONTENT: line[start+1..end-1]
            let raw_content = &line[start + 1..end - 1];

            if !has_esc {
                // FAST PATH: No escapes to handle!
                // We can run FST directly on the raw bytes from the line.
                self.process_content(raw_content)?;
            } else {
                // SLOW PATH: Use Cow to avoid allocation if possible,
                // though has_esc=true means we'll likely allocate once.
                match serde_json::from_slice::<std::borrow::Cow<str>>(&line[start..end]) {
                    Ok(s) => self.process_content(s.as_bytes())?,
                    Err(_) => {
                        if !self.only_matching {
                            self.buf.write_all(&line[start..end])?;
                        }
                    }
                }
            }
            lastpos = end;
        }
        if !self.only_matching {
            self.buf.write_all(&line[lastpos..])?;
        }
        Ok(true)
    }

    #[inline]
    fn process_content(&mut self, input: &[u8]) -> Result<(), io::Error> {
        self.temp_buf.clear();
        let mut inner_lastpos = 0;
        let mut matcher = self.fsed.find_iter(input, self.cache);

        while let Some(m) = matcher.next() {
            self.temp_buf.write_all(&input[inner_lastpos..m])?;
            self.temp_buf
                .write_all(matcher.get_match().render().as_bytes())?;
            inner_lastpos = m + matcher.get_match_len();
        }
        self.temp_buf.write_all(&input[inner_lastpos..])?;

        serde_json::to_writer(&mut self.buf, std::str::from_utf8(&self.temp_buf).unwrap())?;
        Ok(())
    }
}
