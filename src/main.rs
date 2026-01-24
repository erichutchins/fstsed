use anyhow::{bail, Result};
use bstr::io::BufReadExt;
use camino::Utf8PathBuf;
use clap::Parser;
use fstsed::cli::{Args, ArgsColorChoice};
use fstsed::{FstCache, FstSed, BUFFERSIZE};
use ignore::{DirEntry, WalkBuilder};
use std::fs::File;
use std::io::{self, BufReader};
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
    let colormode = determine_color_mode(&args);

    // Initialize FstSed and wrap in Arc for sharing across threads
    let fsed = Arc::new(
        FstSed::new(&args.fst, args.template.clone(), colormode).map_err(anyhow::Error::from)?,
    );

    // For stdout output
    let writer = Arc::new(BufferWriter::stdout(colormode));

    // Differentiate between stdin and file paths
    let has_stdin = args.input.iter().any(|p| p == "-");

    if has_stdin {
        process_stdin(&fsed, &writer, &args)?;
    } else {
        process_files_parallel(&fsed, &writer, &args)?;
    }

    Ok(())
}

fn determine_color_mode(args: &Args) -> ColorChoice {
    match args.color {
        ArgsColorChoice::Auto => ColorChoice::Auto,
        ArgsColorChoice::Always => ColorChoice::Always,
        ArgsColorChoice::Never => ColorChoice::Never,
    }
}

fn process_stdin(fsed: &FstSed, writer: &BufferWriter, args: &Args) -> Result<()> {
    let mut buf = writer.buffer();
    let mut cache = FstCache::new();
    let stdin = io::stdin();
    let mut reader = BufReader::with_capacity(BUFFERSIZE, stdin.lock());

    fsed.stream_find(
        &mut reader,
        &mut buf,
        &mut cache,
        args.only_matching,
        args.json,
    )?;

    writer.print(&buf)?;
    Ok(())
}

fn process_files_parallel(
    fsed: &Arc<FstSed>,
    writer: &Arc<BufferWriter>,
    args: &Args,
) -> Result<()> {
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

    let only_matching = args.only_matching;
    let json_mode = args.json;

    builder.build_parallel().run(|| {
        let fsed = fsed.clone();
        let writer = writer.clone();

        Box::new(move |result| process_entry(result, &fsed, &writer, only_matching, json_mode))
    });

    Ok(())
}

fn process_entry(
    result: std::result::Result<DirEntry, ignore::Error>,
    fsed: &FstSed,
    writer: &BufferWriter,
    only_matching: bool,
    json_mode: bool,
) -> ignore::WalkState {
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

    let mut buf = writer.buffer();
    let mut cache = FstCache::new();
    let mut reader = BufReader::with_capacity(BUFFERSIZE, file);

    if let Err(err) = fsed.stream_find(&mut reader, &mut buf, &mut cache, only_matching, json_mode)
    {
        eprintln!("{}: {err}", entry.path().display());
    }

    let _ = writer.print(&buf);

    ignore::WalkState::Continue
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

    fstsed::builder::build_fstsed(reader, &args.key.unwrap(), &args.fst, args.sorted)
        .map_err(anyhow::Error::from)
}
