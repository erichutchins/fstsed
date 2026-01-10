use camino::Utf8PathBuf;
use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[clap(
    author,
    version,
    about,
    long_about = None,
    after_help = "Examples:
  Build an FST database from JSON:
    fstsed --build -f data.fst -k key data.json

  Basic find and replace:
    echo \"match\" | fstsed -f data.fst

  Use a template for decoration:
    echo \"match\" | fstsed -f data.fst --template \"{key} ({info})\"

  JSON search mode (only search inside quoted JSON strings):
    fstsed -f data.fst --json input.json"
)]
#[allow(clippy::struct_excessive_bools)]
pub struct Args {
    /// Show only nonempty parts of lines that match
    #[clap(short, long)]
    pub only_matching: bool,

    /// Use markers to highlight the matching strings
    #[clap(short = 'C', long, value_enum, default_value_t = ArgsColorChoice::Auto)]
    pub color: ArgsColorChoice,

    /// Specify fst db to use in search or to create in build mode
    #[clap(short = 'f', value_name = "FST", value_hint = clap::ValueHint::FilePath)]
    pub fst: Utf8PathBuf,

    /// Build mode. Build a fst from json data instead of querying one. Specify output path with
    /// the -f --fst parameter. Only first file input parameter or stdin is used to make
    /// the fst
    #[clap(long)]
    pub build: bool,

    /// When building a fst, extract the given json field to use as the key in the fst database.
    /// Key may also be provided as a jsonpointer, e.g. /obj/array/1/item
    #[clap(short = 'k', long, value_name = "KEY", default_value = "key")]
    pub key: Option<String>,

    /// When building a fst, set this if the keys of input json are already lexicographically sorted.
    /// This will make build construction much faster. If this is set but the keys are not sorted,
    /// the fst creation will error
    #[clap(long)]
    pub sorted: bool,

    /// Specify the format of the fstsed match decoration. Field names are enclosed in {},
    /// for example "{field1} any fixed string {field2} & {field3}". Fields may be json keys
    /// or jsonpointers {/obj/array/1/item}
    #[clap(short, long)]
    pub template: Option<String>,

    /// Json search mode. Fstsed will treat input as json, searching only inside quoted json strings.
    /// All strings are deserialized/decoded before json before searching, and all template
    /// decorations are properly json-encoded in the output for subsequent processing
    #[clap(short, long)]
    pub json: bool,

    /// Input file(s) or directory(s) to process. Leave empty or use "-" to read from stdin.
    /// (In build mode, only the first path is used).
    #[clap(value_name = "PATH", value_hint = clap::ValueHint::AnyPath)]
    pub input: Vec<Utf8PathBuf>,

    /// The number of threads to use for searching.
    #[clap(short = 'w', long, value_name = "NUM")]
    pub threads: Option<usize>,

    /// Do not respect ignore files (.gitignore, .ignore, etc.)
    #[clap(short = 'u', long)]
    pub no_ignore: bool,

    /// Search hidden files and directories.
    #[clap(long)]
    pub hidden: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, ValueEnum)]
pub enum ArgsColorChoice {
    Always,
    Never,
    Auto,
}
