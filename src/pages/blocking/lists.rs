use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use adblock::lists::{FilterSet, ParseOptions};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use ulid::Ulid;
use url::Url;

use crate::{address::web_url, atomic_file};

pub(super) const VARIABLE: &str = "CANOPY_BLOCKLISTS";
const MANIFEST_FILE: &str = "lists.json";
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MAX_REDIRECTS: usize = 3;

#[derive(Clone, Debug)]
pub(super) struct Source {
    pub id: String,
    pub url: Url,
}

impl Source {
    fn new(url: Url) -> Self {
        let stem = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .and_then(|file| Path::new(file).file_stem())
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("list");
        let stem: String = stem
            .chars()
            .take(40)
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
                _ => '_',
            })
            .collect();
        let digest = format!("{:x}", Sha256::digest(url.as_str()));
        Self {
            id: format!("{stem}-{}", &digest[..8]),
            url,
        }
    }

    pub(super) fn name(&self) -> &str {
        self.id.rsplit_once('-').unwrap().0
    }

    pub(super) fn next_identifier(&self) -> String {
        format!("{}-{}", self.id, Ulid::new())
    }
}

pub(super) fn configured() -> Vec<Source> {
    let value = std::env::var_os(VARIABLE);
    match parse_sources(value.as_deref().and_then(|value| value.to_str())) {
        Ok(sources) => sources,
        Err(error) => {
            eprintln!("Canopy: invalid {VARIABLE}: {error}");
            Vec::new()
        }
    }
}

fn parse_sources(value: Option<&str>) -> Result<Vec<Source>> {
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for address in value.unwrap_or("").split_whitespace() {
        let mut url = web_url(address)?;
        anyhow::ensure!(
            url.username().is_empty() && url.password().is_none(),
            "List addresses cannot contain credentials."
        );
        url.set_fragment(None);
        if seen.insert(url.clone()) {
            sources.push(Source::new(url));
        }
    }
    Ok(sources)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Record {
    pub identifier: String,
    compiled_at: u64,
}

impl Record {
    pub(super) fn new(identifier: String, now: SystemTime) -> Self {
        Self {
            identifier,
            compiled_at: now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Manifest(pub BTreeMap<String, Record>);

impl Manifest {
    pub(super) fn load(folder: &Path) -> Self {
        fs::read(folder.join(MANIFEST_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .as_ref()
            .and_then(Value::as_object)
            .map(|rows| {
                Self(
                    rows.iter()
                        .filter_map(|(source, row)| {
                            let identifier = row["identifier"].as_str()?;
                            let (prefix, stamp) = identifier.rsplit_once('-')?;
                            if prefix != source
                                || !prefix
                                    .bytes()
                                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
                                || stamp.parse::<Ulid>().is_err()
                            {
                                return None;
                            }
                            Some((
                                source.clone(),
                                Record {
                                    identifier: identifier.to_owned(),
                                    compiled_at: row["compiled_at"].as_u64()?,
                                },
                            ))
                        })
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    pub(super) fn save(&self, folder: &Path) -> Result<()> {
        let rows: BTreeMap<_, _> = self
            .0
            .iter()
            .map(|(source, row)| {
                (
                    source,
                    json!({"identifier": row.identifier, "compiled_at": row.compiled_at}),
                )
            })
            .collect();
        atomic_file::write(&folder.join(MANIFEST_FILE), serde_json::to_vec(&rows)?)?;
        Ok(())
    }
}

pub(super) fn due(record: Option<&Record>, now: SystemTime) -> bool {
    record.is_none_or(|record| {
        let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        since_epoch.saturating_sub(Duration::from_secs(record.compiled_at)) > MAX_AGE
    })
}

pub(super) struct Converted {
    pub json: Vec<u8>,
    pub rules: usize,
}

pub(super) fn convert(body: Vec<u8>) -> Result<Converted> {
    let text = String::from_utf8(body).context("The list is not UTF-8.")?;
    anyhow::ensure!(
        text.trim_start_matches('\u{feff}')
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("[Adblock") && line.trim_end().ends_with(']')),
        "The response has no Adblock list header."
    );
    let mut filters = FilterSet::new(true);
    filters.add_filter_list(text, ParseOptions::default());
    let (rules, _) = filters
        .into_content_blocking()
        .map_err(|()| anyhow::anyhow!("Could not convert the list to WebKit rules."))?;
    anyhow::ensure!(!rules.is_empty(), "The list has no usable rules.");
    Ok(Converted {
        json: serde_json::to_vec(&rules)?,
        rules: rules.len(),
    })
}

pub(super) fn next_hop(
    from: &Url,
    status: u16,
    location: Option<&str>,
    redirects: usize,
) -> Result<Option<Url>> {
    match status {
        200 => Ok(None),
        301 | 302 | 303 | 307 | 308 => {
            anyhow::ensure!(redirects < MAX_REDIRECTS, "Too many list redirects.");
            let mut next = from.join(location.context("The list redirect has no address.")?)?;
            anyhow::ensure!(
                matches!(next.scheme(), "http" | "https")
                    && !(from.scheme() == "https" && next.scheme() == "http")
                    && next.username().is_empty()
                    && next.password().is_none(),
                "The list redirect used an unsupported address."
            );
            next.set_fragment(None);
            Ok(Some(next))
        }
        _ => bail!("HTTP {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_and_empty_mean_no_lists() {
        assert!(parse_sources(None).unwrap().is_empty());
        assert!(parse_sources(Some(" \t\n ")).unwrap().is_empty());
    }

    #[test]
    fn invalid_sources_mean_no_lists() {
        for invalid in ["oops", "file:///a", "https://user:password@example.org/a"] {
            assert!(parse_sources(Some(&format!("https://example.org {invalid}"))).is_err());
        }
    }

    #[test]
    fn list_ids_are_stable_and_safe() {
        let sources = parse_sources(Some(
            "http://a.test/-list.css http://a.test/-list.css#x https://b.test/-list.css",
        ))
        .unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].name(), "-list");
        assert_ne!(sources[0].id, sources[1].id);
        assert_eq!(Source::new(sources[0].url.clone()).id, sources[0].id);
        assert_ne!(sources[0].next_identifier(), sources[0].next_identifier());
    }

    #[test]
    fn due_after_seven_days_or_unknown() {
        let compiled = UNIX_EPOCH + MAX_AGE;
        let item = Record::new("list".into(), compiled);
        assert!(due(None, compiled));
        assert!(!due(Some(&item), UNIX_EPOCH));
        assert!(!due(Some(&item), compiled + MAX_AGE));
        assert!(due(
            Some(&item),
            compiled + MAX_AGE + Duration::from_secs(1)
        ));
    }

    #[test]
    fn manifest_keeps_only_valid_records() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut manifest = Manifest::default();
        let source = Source::new(web_url("https://example.org/list.txt")?);
        let item = Record::new(source.next_identifier(), UNIX_EPOCH + MAX_AGE);
        manifest.0.insert(source.id.clone(), item.clone());
        manifest
            .0
            .insert("bad".into(), Record::new("../bad".into(), UNIX_EPOCH));
        manifest.save(dir.path())?;
        assert_eq!(
            Manifest::load(dir.path()).0,
            BTreeMap::from([(source.id, item)])
        );
        Ok(())
    }

    #[test]
    fn convert_makes_block_and_hide_rules() {
        let converted =
            convert(b"[Adblock Plus 2.0]\n/ads/probe.js$script\n##.adbox\n".to_vec()).unwrap();
        let rules: Value = serde_json::from_slice(&converted.json).unwrap();
        assert_eq!(converted.rules, rules.as_array().unwrap().len());
        assert!(rules.as_array().unwrap().iter().any(|rule| {
            rule["action"]["type"] == "block"
                && rule["trigger"]["url-filter"]
                    .as_str()
                    .unwrap()
                    .contains("probe")
        }));
        assert!(rules.as_array().unwrap().iter().any(|rule| {
            rule["action"]["type"] == "css-display-none" && rule["action"]["selector"] == ".adbox"
        }));
    }

    #[test]
    fn convert_refuses_pages_and_empty_lists() {
        for body in [
            b"<!doctype html><title>Page</title>".as_slice(),
            b"[Adblock Plus 2.0]\n! empty\n",
            b"\xff",
        ] {
            assert!(convert(body.to_vec()).is_err());
        }
    }

    #[test]
    fn redirects_follow_up_to_three_never_down_to_http() {
        let url = web_url("https://example.org/a").unwrap();
        assert_eq!(next_hop(&url, 200, None, 3).unwrap(), None);
        assert_eq!(
            next_hop(&url, 302, Some("new.txt"), 2)
                .unwrap()
                .unwrap()
                .as_str(),
            "https://example.org/new.txt"
        );
        for (status, to, redirects) in [
            (302, Some("http://example.org/to"), 0),
            (302, Some("file:///to"), 0),
            (302, Some("/to"), 3),
            (302, None, 0),
            (500, None, 0),
        ] {
            assert!(next_hop(&url, status, to, redirects).is_err());
        }
    }
}
