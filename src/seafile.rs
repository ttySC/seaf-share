use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use url::Url;

use super::DirEntry;

#[derive(Debug)]
pub enum Error {
    InvalidShare,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidShare => write!(f, "invalid share"),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebFileOptions {
    #[serde(rename = "repoID")]
    repo_id: String,
    #[serde(rename = "filePath")]
    path: PathBuf,
    #[serde(rename = "fileName")]
    name: String,
    #[serde(rename = "fileSize")]
    size: u64,
    raw_path: Url,
    can_download: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebPageOptions<T> {
    #[serde(rename = "pageOptions")]
    options: T,
}

// TODO: the enum can be tagged by `is_dir` once these issues are resolved
//
// https://github.com/serde-rs/serde/issues/745
// https://github.com/serde-rs/serde/issues/880
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all_fields = "snake_case")]
pub enum DirEnt {
    Directory {
        is_dir: bool,
        last_modified: DateTime<Utc>,
        #[serde(rename = "folder_path")]
        path: PathBuf,
        #[serde(rename = "folder_name")]
        name: String,
        size: u64,
    },
    File {
        is_dir: bool,
        last_modified: DateTime<Utc>,
        #[serde(rename = "file_path")]
        path: PathBuf,
        #[serde(rename = "file_name")]
        name: String,
        size: u64,
        encoded_thumbnail_src: Option<PathBuf>,
    },
}

impl DirEnt {
    pub fn is_file(&self) -> bool {
        match self {
            Self::Directory { .. } => false,
            Self::File { .. } => true,
        }
    }

    pub fn is_dir(&self) -> bool {
        !self.is_file()
    }

    pub fn size(&self) -> Option<u64> {
        match self {
            Self::Directory { .. } => None,
            Self::File { size, .. } => Some(*size),
        }
    }

    pub fn last_modified(&self) -> &DateTime<Utc> {
        match self {
            Self::Directory { last_modified, .. } | Self::File { last_modified, .. } => {
                last_modified
            }
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Directory { name, .. } | Self::File { name, .. } => name,
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Directory { path, .. } | Self::File { path, .. } => path.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct DirEntList {
    #[serde(rename = "dirent_list")]
    entries: Vec<DirEnt>,
}

pub struct Client {
    client: ureq::Agent,
    base: Url,
    quickjs: rquickjs::Runtime,
}

impl Client {
    pub fn with_agent(agent: ureq::Agent, url: &Url) -> Self {
        let mut base = url.clone();
        base.set_path("");
        base.set_query(None);
        Self {
            client: agent,
            base,
            quickjs: rquickjs::Runtime::new().unwrap(),
        }
    }

    fn dir_url(&self, token: impl AsRef<str>, path: Option<impl AsRef<Path>>) -> Url {
        let mut url = self.base.clone();
        url.set_path(&format!("/d/{}/", token.as_ref()));
        if let Some(path) = path {
            path.as_ref().to_str().map(|p| {
                url.query_pairs_mut().append_pair("p", p);
            });
        }
        url
    }

    fn file_url(&self, token: impl AsRef<str>, path: impl AsRef<Path>, dl: bool) -> Url {
        let mut url = self.base.clone();
        url.set_path(&format!("/d/{}/files/", token.as_ref()));
        if let Some(p) = path.as_ref().to_str() {
            url.query_pairs_mut().append_pair("p", p);
        }
        if dl {
            url.query_pairs_mut().append_pair("dl", "1");
        }
        url
    }

    // https://download.seafile.com/published/web-api/v2.1/share-links.md
    pub fn api_dirents(
        &self,
        token: impl AsRef<str>,
        path: Option<impl AsRef<Path>>,
    ) -> anyhow::Result<Vec<DirEnt>> {
        let mut url = self.base.clone();
        url.set_path(&format!(
            "/api/v2.1/share-links/{}/dirents/",
            token.as_ref()
        ));
        if let Some(path) = path {
            path.as_ref().to_str().map(|s| {
                url.query_pairs_mut().append_pair("path", s);
            });
        }
        let mut res = self.client.get(url.as_str()).call()?;
        let list = res.body_mut().read_json::<DirEntList>()?;
        Ok(list.entries)
    }

    fn extract_page_options<T: serde::de::DeserializeOwned>(
        &self,
        page: impl AsRef<str>,
    ) -> Option<T> {
        use rquickjs::{Context, Function, Object, Value};
        let object_pattern = Regex::new(r"window\.shared\s*=\s*(\{[\s\S]*?\});").ok()?;
        let captures = object_pattern.captures(page.as_ref())?;
        let shared = captures.get(0)?.as_str();
        let ctx = Context::full(&self.quickjs).ok()?;
        let ret = ctx
            .with(|ctx| -> rquickjs::Result<String> {
                ctx.globals().set("window", Object::new(ctx.clone())?)?;
                let json: Object = ctx.globals().get("JSON")?;
                let json_stringify: Function = json.get("stringify")?;
                ctx.eval::<Value, _>(shared)
                    .and_then(|v| json_stringify.call::<(Value<'_>,), rquickjs::String>((v,)))
                    .and_then(|s| s.to_string())
            })
            .ok()?;
        let page_options: WebPageOptions<T> = serde_json::from_str(ret.as_ref()).ok()?;
        Some(page_options.options)
    }

    pub fn web_file(&self, url: &Url) -> anyhow::Result<WebFileOptions> {
        let mut res = self.client.get(url.as_str()).call()?;
        let body = res.body_mut().read_to_string()?;
        Ok(self.extract_page_options(body).ok_or(Error::InvalidShare)?)
    }

    pub fn entries(
        &self,
        token: impl AsRef<str>,
        path: Option<impl AsRef<Path>>,
    ) -> anyhow::Result<Vec<DirEntry>> {
        let dirents = self.api_dirents(token.as_ref(), path)?;
        let entries = dirents
            .iter()
            .map(|e| {
                if e.is_file() {
                    DirEntry::File {
                        name: e.name().to_string(),
                        path: e.path().to_path_buf(),
                        size: e.size().unwrap(),
                        last_modified: Some(e.last_modified().clone()),
                        view_url: self.file_url(token.as_ref(), e.path(), false),
                        download_url: self.file_url(token.as_ref(), e.path(), true),
                    }
                } else if e.is_dir() {
                    DirEntry::Directory {
                        name: e.name().to_string(),
                        path: e.path().to_path_buf(),
                        last_modified: e.last_modified().clone(),
                        view_url: self.dir_url(token.as_ref(), Some(e.path())),
                    }
                } else {
                    unreachable!()
                }
            })
            .collect();
        Ok(entries)
    }

    pub fn single_file(&self, url: &Url) -> anyhow::Result<DirEntry> {
        let file = self.web_file(url)?;
        let entry = DirEntry::File {
            name: file.name.clone(),
            path: file.path.clone(),
            size: file.size,
            last_modified: None,
            view_url: url.clone(),
            download_url: file.raw_path.clone(),
        };
        Ok(entry)
    }
}
