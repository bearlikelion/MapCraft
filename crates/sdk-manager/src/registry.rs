use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Error, MAPCRAFT_VERSION, io_err};

/// The official registry.
pub const DEFAULT_REGISTRY: &str = "https://raw.githubusercontent.com/bearlikelion/MapCraftSDKs/main/index.json";

/// The `format` tag every registry carries.
pub const FORMAT: &str = "mapcraft.sdk-registry";
/// The newest registry version this MapCraft reads.
pub const VERSION: u32 = 1;

const MAX_INDEX: u64 = 8 << 20;
/// Largest SDK archive MapCraft downloads.
pub(crate) const MAX_DOWNLOAD: u64 = 512 << 20;

/// A registry file (`index.json`): a list of SDKs and where to download them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sdks: Vec<Entry>,
}

/// One SDK in a registry. Unknown keys are ignored, so later versions can add some.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// SPDX expression, such as `MIT`.
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub homepage: String,
    /// The zip's URL, or a path relative to the registry file.
    pub download: String,
    /// SHA-256 of the zip, in hex.
    pub sha256: String,
    /// Size of the zip in bytes.
    pub size: u64,
    /// The oldest MapCraft version the SDK works with.
    #[serde(default)]
    pub min_mapcraft: String,
    /// Ids of the plugins the zip ships.
    #[serde(default)]
    pub plugins: Vec<String>,
}

impl Registry {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let reg: Registry = serde_json::from_str(text).map_err(|e| Error::Registry(e.to_string()))?;
        if reg.format != FORMAT {
            return Err(Error::Registry(format!("`format` is `{}`, expected `{FORMAT}`", reg.format)));
        }
        if reg.version > VERSION {
            return Err(Error::Registry(format!("version {} needs a newer MapCraft (this one reads up to {VERSION})", reg.version)));
        }
        let mut ids = BTreeSet::new();
        for e in &reg.sdks {
            let bad = |why: &str| Err(Error::Registry(format!("SDK `{}`: {why}", e.id)));
            if !mapcraft_sdk::valid_id(&e.id) {
                return bad("the id must use lowercase letters, digits, '_' or '-'");
            }
            if !ids.insert(e.id.as_str()) {
                return bad("listed twice");
            }
            if e.name.trim().is_empty() || e.version.trim().is_empty() || e.download.trim().is_empty() {
                return bad("`name`, `version` and `download` must not be empty");
            }
            if e.sha256.len() != 64 || !e.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return bad("`sha256` must be 64 hex digits");
            }
            if e.size == 0 || e.size > MAX_DOWNLOAD {
                return bad("`size` must be between 1 byte and 512 MiB");
            }
        }
        Ok(reg)
    }
}

impl Entry {
    /// Where the zip downloads from: `download` itself when it is a URL, else relative to `registry`.
    /// Entries of a remote registry always resolve to URLs, never to local files.
    pub fn source(&self, registry: &str) -> String {
        if is_url(&self.download) {
            return self.download.clone();
        }
        if is_url(registry) {
            let base = registry.rsplit_once('/').map_or(registry, |(b, _)| b);
            return format!("{base}/{}", self.download.trim_start_matches('/'));
        }
        // Keeping the registry's trailing separator stops join adding a native one, so `/` paths stay `/` on Windows.
        let dir = registry.strip_prefix("file://").unwrap_or(registry).trim_end_matches(|c| !std::path::is_separator(c));
        Path::new(dir).join(&self.download).to_string_lossy().into_owned()
    }

    /// Why this MapCraft cannot install the entry, if it cannot.
    pub fn unsupported(&self) -> Option<String> {
        let min = self.min_mapcraft.trim();
        (!min.is_empty() && crate::newer(min, MAPCRAFT_VERSION)).then(|| format!("needs MapCraft {min} or newer"))
    }
}

fn is_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
}

fn local(s: &str) -> PathBuf {
    PathBuf::from(s.strip_prefix("file://").unwrap_or(s))
}

/// Reads and parses a registry from an `http(s)://` URL or a local path.
pub fn fetch_registry(source: &str) -> Result<Registry, Error> {
    let bytes = fetch(source, MAX_INDEX, &mut |_, _| {}, &AtomicBool::new(false))?;
    Registry::parse(&String::from_utf8_lossy(&bytes))
}

fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .user_agent(format!("MapCraft/{MAPCRAFT_VERSION}"))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(600)))
        .build()
        .new_agent()
}

/// Reads up to `limit` bytes from an `http(s)://` URL or a local path, reporting (bytes so far,
/// total when known) and stopping when `cancel` is set.
pub fn fetch(source: &str, limit: u64, progress: &mut dyn FnMut(u64, Option<u64>), cancel: &AtomicBool) -> Result<Vec<u8>, Error> {
    let too_large = || Error::TooLarge(format!("{source} is larger than {} MiB", limit >> 20));
    if !is_url(source) {
        let path = local(source);
        let len = std::fs::metadata(&path).map_err(io_err(&path))?.len();
        if len > limit {
            return Err(too_large());
        }
        let bytes = std::fs::read(&path).map_err(io_err(&path))?;
        progress(bytes.len() as u64, Some(len));
        return Ok(bytes);
    }
    let failed = |e: &dyn std::fmt::Display| Error::Download { url: source.to_string(), message: e.to_string() };
    let mut response = agent().get(source).call().map_err(|e| failed(&e))?;
    let total = response.body().content_length();
    if total.is_some_and(|t| t > limit) {
        return Err(too_large());
    }
    let mut reader = response.body_mut().with_config().limit(limit.saturating_add(1)).reader();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| failed(&e))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(buf.get(..n).unwrap_or_default());
        if out.len() as u64 > limit {
            return Err(too_large());
        }
        progress(out.len() as u64, total);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.into(),
            name: "Demo".into(),
            version: "1.0".into(),
            description: String::new(),
            author: "Someone".into(),
            license: "MIT".into(),
            homepage: String::new(),
            download: format!("{id}-1.0.zip"),
            sha256: "0".repeat(64),
            size: 10,
            min_mapcraft: "0.1.0".into(),
            plugins: vec![],
        }
    }

    #[test]
    fn parses_and_validates() {
        let text = r#"{
            "format": "mapcraft.sdk-registry", "version": 1, "name": "Test",
            "sdks": [{ "id": "demo", "name": "Demo", "version": "1.0", "download": "demo-1.0.zip",
                       "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                       "size": 10, "min_mapcraft": "0.1.0", "plugins": ["demo"], "future_key": true }]
        }"#;
        let reg = Registry::parse(text).unwrap();
        assert_eq!(reg.sdks[0].plugins, ["demo"]);
        assert_eq!(reg.sdks[0].source("https://example.com/sdks/index.json"), "https://example.com/sdks/demo-1.0.zip");
        assert_eq!(reg.sdks[0].source("/srv/reg/index.json"), "/srv/reg/demo-1.0.zip");
        assert_eq!(reg.sdks[0].source("index.json"), "demo-1.0.zip");
        #[cfg(windows)]
        assert_eq!(reg.sdks[0].source(r"C:\reg\index.json"), r"C:\reg\demo-1.0.zip");
        assert!(reg.sdks[0].unsupported().is_none());

        let mut far = entry("far");
        far.min_mapcraft = "99.0".into();
        assert!(far.unsupported().is_some());
        let mut local_file = entry("x");
        local_file.download = "/etc/passwd".into();
        assert!(local_file.source("https://example.com/index.json").starts_with("https://example.com/"), "a remote registry never reads local files");

        assert!(Registry::parse(&text.replace("mapcraft.sdk-registry", "other")).is_err());
        assert!(Registry::parse(&text.replace("\"version\": 1", "\"version\": 2")).is_err());
        assert!(Registry::parse(&text.replace("\"demo\", \"name\"", "\"../demo\", \"name\"")).is_err());
        assert!(Registry::parse(&text.replace("\"size\": 10", "\"size\": 0")).is_err());
        assert!(Registry::parse(&text.replace("00000000000000000000\"", "0000000000000000000z\"")).is_err());
        assert!(Registry::parse("not json").is_err());
    }
}
