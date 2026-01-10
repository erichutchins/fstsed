use anyhow::{bail, Result};
use camino::Utf8PathBuf;
use clap::Parser;
use fstsed::cli::{Args, ArgsColorChoice};
use fstsed::fstsed::{FstCache, FstSed};
use fstsed::sink::FstSedSink;
use fstsed::BUFFERSIZE;
use grep_regex::RegexMatcher;
use grep_searcher::Searcher;
use ignore::WalkBuilder;
use std::io::{self, IsTerminal};
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
    let fsed = Arc::new(FstSed::new(args.fst, args.template, colormode)?);

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

        let sink = FstSedSink::new(&fsed, &mut cache, &mut buf, args.only_matching, args.json);

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
                        eprintln!("{err}");
                        return ignore::WalkState::Continue;
                    }
                };

                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    return ignore::WalkState::Continue;
                }

                let sink =
                    FstSedSink::new(&fsed, &mut cache, &mut buf, args.only_matching, args.json);

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
