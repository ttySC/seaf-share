use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use url::Url;

#[derive(Debug, Clone, Parser)]
#[clap(version)]
pub struct Cli {
    #[clap(subcommand)]
    command: Command,
}

impl Cli {
    pub fn command(&self) -> &Command {
        &self.command
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    List(ListOptions),
    Download(DownloadOptions),
}

impl Command {
    pub fn common(&self) -> &CommonOptions {
        match self {
            Self::List(options) => options.common(),
            Self::Download(options) => options.common(),
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct ListOptions {
    #[clap(flatten)]
    common: CommonOptions,
    /// JSON output
    #[clap(long)]
    json: bool,
}

impl ListOptions {
    pub fn common(&self) -> &CommonOptions {
        &self.common
    }
    pub fn json(&self) -> bool {
        self.json
    }
}

#[derive(Debug, Clone, Args)]
pub struct CommonOptions {
    /// Seafile share URL (subfolder URL is also supported, see examples with "--help")
    ///
    /// Examples:
    /// https://cloud.example/d/abc
    /// https://cloud.example/f/abc
    /// https://cloud.example/d/6e5297246c/?p=%2Fpath&mode=list
    /// https://cloud.example/d/6e5297246c/files/?p=%2Fpath%2Ffile.jpg
    #[clap(verbatim_doc_comment)]
    url: Url,

    /// Remote path to fetch, which can be absolute or relative to the share URL
    #[clap(short, long)]
    path: Option<PathBuf>,
}

impl CommonOptions {
    pub fn url(&self) -> &Url {
        &self.url
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_ref().map(|p| p.as_ref())
    }
}

#[derive(Debug, Clone, Args)]
pub struct DownloadOptions {
    #[clap(flatten)]
    common: CommonOptions,

    /// Dry run (output download link only)
    #[clap(long)]
    dry_run: bool,

    /// Output destination
    #[clap(short, long, default_value = "./")]
    output: PathBuf,

    /// Archive mode, which sets "mtime" (modification time) shown in remote
    #[clap(short, long)]
    archive: bool,

    /// Action to be taken if a file already exists
    #[clap(short, long, default_value_t, value_enum)]
    conflict: ConflictAction,

    /// Include remote paths only (GLOB patterns, see examples with "--help")
    ///
    /// Examples:
    /// /xyz/*
    /// /ab?/**
    ///
    /// Check https://docs.rs/glob/latest/glob/struct.Pattern.html for details.
    #[clap(long)]
    include: Vec<glob::Pattern>,

    /// Exclude remote paths (GLOB patterns)
    #[clap(long)]
    exclude: Vec<glob::Pattern>,

    /// Recursive download (DFS by default)
    #[clap(
        short, long,
        require_equals = true, num_args = 0..=1, default_missing_value = "dfs",
        default_value_t, value_enum,
    )]
    recursive: Recursive,
}

impl DownloadOptions {
    pub fn common(&self) -> &CommonOptions {
        &self.common
    }
    pub fn dry_run(&self) -> bool {
        self.dry_run
    }
    pub fn output(&self) -> &Path {
        self.output.as_ref()
    }
    pub fn archive(&self) -> bool {
        self.archive
    }
    pub fn on_conflict(&self) -> ConflictAction {
        self.conflict
    }
    pub fn includes(&self) -> &[glob::Pattern] {
        self.include.as_slice()
    }
    pub fn excludes(&self) -> &[glob::Pattern] {
        self.exclude.as_slice()
    }
    pub fn recursive(&self) -> Recursive {
        self.recursive
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, ValueEnum)]
pub enum ConflictAction {
    /// Skip if a file exists
    #[default]
    Skip,

    /// Verify by downloading remote chunks in memory, overwrite if the checksum
    /// is not correct.
    Check,

    /// Continue the download by sending partial requests ("Range" header).
    Continue,

    /// always overwrite the destination
    Overwrite,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, ValueEnum)]
pub enum Recursive {
    /// Do not look into subdirectory entries
    #[default]
    None,

    /// Traverse subdirectories by DFS
    Dfs,

    /// Traverse subdirectories by BFS
    Bfs,
}
