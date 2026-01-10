use anyhow::{bail, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use clap::Parser;
use fstsed::cli::{Args, ArgsColorChoice};
use fstsed::fstsed::{FstCache, FstSed};
use fstsed::sink::FstSedSink;
use fstsed::BUFFERSIZE;
use ignore::WalkBuilder;
use std::fs::File;
use std::io::{self, BufReader, IsTerminal};
use std::path::Path;
use std::sync::Arc;
use termcolor::{BufferWriter, ColorChoice};

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
    let fsed = Arc::new(
        FstSed::new(args.fst, args.template, colormode).map_err(|e| anyhow::anyhow!("{}", e))?,
    );

    // For stdout output
    let writer = Arc::new(BufferWriter::stdout(colormode));

    // Differentiate between stdin and file paths
    let has_stdin = args.input.iter().any(|p| p == "-");

    if has_stdin {
        // Single threaded stdin processing
        let mut buf = writer.buffer();
        let mut cache = FstCache::new();
        let mut sink = FstSedSink::new(&fsed, &mut cache, &mut buf, args.only_matching);

        let stdin = io::stdin();
        let mut reader = BufReader::with_capacity(BUFFERSIZE, stdin.lock());

        reader.for_byte_line_with_terminator(|line| {
            if args.json {
                sink.process_json(line)?;
            } else {
                sink.process_text(line)?;
            }
            Ok(true)
        })?;

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

            Box::new(move |result| {
                let entry = match result {
                    Ok(entry) => entry,
                    Err(err) => {
                        eprintln!("{err}");
                        return ignore::WalkState::Continue;
                    }
                };

                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }

                let file = match File::open(entry.path()) {
                    Ok(f) => f,
                    Err(err) => {
                        eprintln!("{}: {err}", entry.path().display());
                        return ignore::WalkState::Continue;
                    }
                };

                let mut reader = BufReader::with_capacity(BUFFERSIZE, file);
                let mut sink = FstSedSink::new(&fsed, &mut cache, &mut buf, args.only_matching);

                let result = reader.for_byte_line_with_terminator(|line| {
                    if args.json {
                        sink.process_json(line)?;
                    } else {
                        sink.process_text(line)?;
                    }
                    Ok(true)
                });

                if let Err(err) = result {
                    eprintln!("{}: {err}", entry.path().display());
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
    let reader: Box<dyn BufReadExt> = if input == "-" {
        Box::new(BufReader::with_capacity(BUFFERSIZE, io::stdin()))
    } else {
        Box::new(BufReader::with_capacity(BUFFERSIZE, File::open(input)?))
    };

    fstsed::build::build_fstsed(reader, &args.key.unwrap(), &args.fst, args.sorted)
        .map_err(|e| anyhow::anyhow!("{}", e))
}
