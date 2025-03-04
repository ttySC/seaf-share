mod cli;
mod seafile;

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use clap::Parser;
use cli_table::{Cell, Table};
use human_bytes::human_bytes;
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use url::Url;

use cli::{Cli, Command, ConflictAction, DownloadOptions, Recursive};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum DownloadResult {
    Skipped,
    Overwritten,
    Continued,
    Complete,
}

impl std::fmt::Display for DownloadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => write!(f, "skipped"),
            Self::Overwritten => write!(f, "overwritten"),
            Self::Continued => write!(f, "continued"),
            Self::Complete => write!(f, "complete"),
        }
    }
}

use std::fs::OpenOptions;
fn conflict_file_options(conflict: ConflictAction) -> OpenOptions {
    let mut options = OpenOptions::new();
    match conflict {
        ConflictAction::Skip => {
            options.read(true);
        }
        ConflictAction::Check => {
            options.read(true).write(true);
        }
        ConflictAction::Continue => {
            options.append(true);
        }
        ConflictAction::Overwrite => {
            options.write(true).truncate(true);
        }
    }
    options
}

struct Downloader {
    client: ureq::Agent,
}

impl Downloader {
    fn with_client(client: ureq::Agent) -> Self {
        Self { client }
    }
    fn download<W: ?Sized>(&self, writer: &mut W, url: &Url) -> anyhow::Result<u64>
    where
        W: std::io::Write,
    {
        let mut res = self.client.get(url.as_str()).call()?;
        let mut reader = res.body_mut().as_reader();
        Ok(std::io::copy(&mut reader, writer)?)
    }

    fn download_range<W: ?Sized>(
        &self,
        writer: &mut W,
        url: &Url,
        range: std::ops::Range<u64>,
    ) -> anyhow::Result<u64>
    where
        W: std::io::Write,
    {
        let mut res = self
            .client
            .get(url.as_str())
            .header("range", format!("bytes={}-{}", range.start, range.end - 1))
            .call()?;
        if res.status() == ureq::http::StatusCode::PARTIAL_CONTENT {
            let mut reader = res.body_mut().as_reader();
            Ok(std::io::copy(&mut reader, writer)?)
        } else {
            todo!()
        }
    }

    pub fn download_entry(
        &self,
        entry: &DirEntry,
        options: &DownloadOptions,
    ) -> anyhow::Result<DownloadResult> {
        if entry.is_dir() {
            return Ok(DownloadResult::Skipped);
        }

        let mut dest = options.output().to_path_buf();
        dest.push(entry.path().strip_prefix("/")?);

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let url = entry.download_url().unwrap();

        let (file, result) = if std::fs::exists(&dest)? {
            let action = options.on_conflict();
            let mut file = conflict_file_options(action).open(dest)?;
            let result = match action {
                ConflictAction::Skip => DownloadResult::Skipped,
                ConflictAction::Check => {
                    todo!()
                }
                ConflictAction::Continue => {
                    let start = file.metadata()?.len();
                    let end = entry.size().unwrap();
                    if start < end {
                        self.download_range(&mut file, url, start..end)?;
                        DownloadResult::Continued
                    } else {
                        DownloadResult::Skipped
                    }
                }
                ConflictAction::Overwrite => {
                    self.download(&mut file, url)?;
                    DownloadResult::Overwritten
                }
            };
            (file, result)
        } else {
            let mut file = std::fs::File::create(dest)?;
            self.download(&mut file, url)?;
            (file, DownloadResult::Complete)
        };
        if options.archive() {
            if let Some(mtime) = entry.last_modified() {
                file.set_modified(mtime.clone().into())?;
            }
        }
        Ok(result)
    }
}

#[derive(Debug, Clone)]
enum ShareLink {
    Directory {
        token: String,
        path: Option<PathBuf>,
        file: bool,
    },
    SingleFile {
        token: String,
    },
}

impl ShareLink {
    pub fn token(&self) -> &str {
        match self {
            Self::Directory { token, .. } => token,
            Self::SingleFile { token } => token,
        }
    }
    pub fn is_single_file(&self) -> bool {
        match self {
            Self::Directory { .. } => false,
            Self::SingleFile { .. } => true,
        }
    }
    pub fn is_dir(&self) -> bool {
        !self.is_file()
    }
    pub fn is_file(&self) -> bool {
        match self {
            Self::Directory { file, .. } => *file,
            Self::SingleFile { .. } => true,
        }
    }
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Directory { path, .. } => path.as_ref().map(|p| p.as_ref()),
            Self::SingleFile { .. } => None,
        }
    }
    fn from_url(url: &Url) -> Option<Self> {
        const PATTERNS: &'static [&'static str] = &["/d/([0-9a-f]+)(/files)?", "/f/([0-9a-f]+)"];
        let set = RegexSet::new(PATTERNS).unwrap();
        let result = set.matches(url.path());
        if let Some(idx) = result.iter().next() {
            let pattern = Regex::new(PATTERNS[idx]).unwrap();
            let captures = pattern.captures(url.path()).unwrap();
            let token = captures.get(1).unwrap();
            if idx == 0 {
                let path = url
                    .query_pairs()
                    .find_map(|(k, v)| if k == "p" { Some(v) } else { None });
                let share = ShareLink::Directory {
                    token: token.as_str().to_string(),
                    path: path.and_then(|s| PathBuf::from_str(s.as_ref()).ok()),
                    file: captures.get(2).is_some(),
                };
                Some(share)
            } else {
                let share = ShareLink::SingleFile {
                    token: token.as_str().to_string(),
                };
                Some(share)
            }
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "snake_case"
)]
enum DirEntry {
    Directory {
        name: String,
        path: PathBuf,
        last_modified: DateTime<Utc>,
        view_url: Url,
    },
    File {
        name: String,
        path: PathBuf,
        size: u64,
        last_modified: Option<DateTime<Utc>>,
        download_url: Url,
        view_url: Url,
    },
}

impl DirEntry {
    fn is_file(&self) -> bool {
        match self {
            Self::Directory { .. } => false,
            Self::File { .. } => true,
        }
    }
    fn is_dir(&self) -> bool {
        match self {
            Self::Directory { .. } => true,
            Self::File { .. } => false,
        }
    }
    fn name(&self) -> &str {
        match self {
            Self::Directory { name, .. } | Self::File { name, .. } => name,
        }
    }
    fn path(&self) -> &Path {
        match self {
            Self::Directory { path, .. } | Self::File { path, .. } => path,
        }
    }
    fn size(&self) -> Option<u64> {
        match self {
            Self::Directory { .. } => None,
            Self::File { size, .. } => Some(*size),
        }
    }
    fn last_modified(&self) -> Option<&DateTime<Utc>> {
        match self {
            Self::Directory { last_modified, .. } => Some(last_modified),
            Self::File { last_modified, .. } => last_modified.as_ref(),
        }
    }
    fn download_url(&self) -> Option<&Url> {
        match self {
            Self::Directory { .. } => None,
            Self::File { download_url, .. } => Some(download_url),
        }
    }
    fn view_url(&self) -> &Url {
        match self {
            Self::Directory { view_url, .. } => view_url,
            Self::File { download_url, .. } => download_url,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let command = cli.command();
    let common = command.common();
    if let Some(link) = ShareLink::from_url(common.url()) {
        let proxy = ureq::Proxy::try_from_env();
        if proxy.is_some() {
            eprintln!("{}", "Proxy environment variables are used.");
        }
        let config = ureq::config::Config::builder()
            .proxy(proxy.clone())
            .accept("application/json")
            .build();
        let client =
            seafile::Client::with_agent(ureq::Agent::new_with_config(config), common.url());
        let downloader = Downloader::with_client(ureq::Agent::new_with_config(
            ureq::config::Config::builder().proxy(proxy.clone()).build(),
        ));
        let path = common
            .path()
            .as_ref()
            .map(|p| {
                let base = link.path().unwrap_or(Path::new("/"));
                let mut buf = base.to_path_buf();
                buf.push(p);
                buf
            })
            .or(link.path().map(|p| p.to_path_buf()));

        match command {
            Command::List(options) => {
                let mut result = Vec::new();
                if link.is_single_file() {
                    let file = client
                        .single_file(common.url())
                        .with_context(|| "cannot fetch single file info")?;
                    result.push(file);
                } else if link.is_file() {
                    let parent = link.path().and_then(|p| p.parent());
                    let entries = client.entries(link.token(), parent)?;
                    let file = entries
                        .iter()
                        .find(|e| link.path().map(|p| p == e.path()).unwrap_or(false));
                    if let Some(file) = file {
                        result.push(file.clone());
                    }
                } else {
                    let entries = client.entries(link.token(), path.as_ref())?;
                    result.extend(entries);
                }
                if options.json() {
                    println!("{}", serde_json::to_string(&result)?);
                } else {
                    let table = result
                        .iter()
                        .map(|e| {
                            let name = if e.is_dir() {
                                format!("{}/", e.name())
                            } else {
                                e.name().to_string()
                            };
                            let na = "N/A".to_string();
                            [
                                name.cell(),
                                e.size()
                                    .map(|sz| human_bytes(sz as f64))
                                    .unwrap_or(na.clone())
                                    .cell(),
                                e.last_modified()
                                    .map(|dt| dt.to_rfc3339())
                                    .unwrap_or(na.clone())
                                    .cell(),
                            ]
                        })
                        .table()
                        .title(["Name", "Size", "Last Modified"])
                        .display()?;
                    println!("{}", table);
                }
            }
            Command::Download(options) => {
                let mut queue = VecDeque::new();
                if link.is_file() {
                    let file = if link.is_single_file() {
                        client.single_file(common.url())?
                    } else {
                        let parent = link.path().and_then(|p| p.parent());
                        let entries = client.entries(link.token(), parent)?;
                        let file = entries
                            .iter()
                            .find(|e| link.path().map(|p| p == e.path()).unwrap_or(false));
                        file.expect("remote file should be found in its parent")
                            .clone()
                    };
                    queue.push_back(file);
                } else {
                    let entries = client.entries(link.token(), path.as_ref())?;
                    if options.recursive() == Recursive::Dfs {
                        queue.extend(entries.into_iter().rev());
                    } else {
                        queue.extend(entries);
                    }
                }

                while !queue.is_empty() {
                    let entry = if options.recursive() == Recursive::Dfs {
                        queue.pop_back().unwrap()
                    } else {
                        queue.pop_front().unwrap()
                    };

                    let mut dest = options.output().to_path_buf();
                    if let Some(base) = path.as_ref() {
                        dest.push(entry.path().strip_prefix(base)?);
                    } else {
                        dest.push(entry.path().strip_prefix("/")?);
                    }

                    if options
                        .excludes()
                        .iter()
                        .any(|p| p.matches_path(entry.path()))
                    {
                        continue;
                    }
                    if entry.is_file() {
                        if options.dry_run() {
                            eprintln!("{}", entry.download_url().unwrap());
                        } else {
                            match downloader.download_entry(&entry, options) {
                                Err(e) => {
                                    eprintln!(
                                        "could not download {}: {}",
                                        entry.path().to_string_lossy(),
                                        e,
                                    )
                                }
                                Ok(result) => {
                                    println!(
                                        "downloaded {}: {}",
                                        entry.path().to_string_lossy(),
                                        result
                                    )
                                }
                            }
                        }
                    } else if options.recursive() != Recursive::None {
                        if !options.dry_run() {
                            std::fs::create_dir(dest)?;
                        }
                        let entries = client.entries(link.token(), Some(entry.path()))?;
                        if options.recursive() == Recursive::Dfs {
                            queue.extend(entries.into_iter().rev());
                        } else {
                            queue.extend(entries)
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
