use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value};

const MINIFIED_FORMAT: &str = "composer/2.0";
const UNSET: &str = "__unset";
const VERSION: &str = "version";
const DIST: &str = "dist";
const DIST_URL: &str = "url";

type Entries = Vec<Map<String, Value>>;

#[derive(Deserialize)]
struct Document {
    minified: Option<String>,
    packages: HashMap<String, Entries>,
}

pub struct PackageListing {
    pub name: String,
    versions: Vec<ListedVersion>,
}

pub struct ListedVersion {
    version: String,
    pub dist_url: Option<String>,
}

pub fn read(listing: &str) -> Result<PackageListing, String> {
    let document: Document = serde_json::from_str(listing).map_err(unreadable)?;
    let is_minified = document.minified.as_deref() == Some(MINIFIED_FORMAT);
    let (name, entries) = only_package(document.packages)?;
    Ok(PackageListing {
        name,
        versions: expand(entries, is_minified),
    })
}

fn only_package(packages: HashMap<String, Entries>) -> Result<(String, Entries), String> {
    if packages.len() > 1 {
        return Err(unreadable(format!(
            "it lists {} packages instead of one",
            packages.len()
        )));
    }
    packages
        .into_iter()
        .next()
        .ok_or_else(|| unreadable("it names no package"))
}

fn unreadable(reason: impl std::fmt::Display) -> String {
    format!("Failed to read the Packagist listing: {reason}")
}

fn expand(entries: Entries, is_minified: bool) -> Vec<ListedVersion> {
    let mut carried = Map::new();
    entries
        .into_iter()
        .filter_map(|entry| {
            if !is_minified {
                carried.clear();
            }
            carry_forward(&mut carried, entry);
            listed_version(&carried)
        })
        .collect()
}

fn carry_forward(carried: &mut Map<String, Value>, entry: Map<String, Value>) {
    for (key, value) in entry {
        if ![VERSION, DIST].contains(&key.as_str()) {
            continue;
        }
        if value == UNSET {
            carried.remove(&key);
        } else {
            carried.insert(key, value);
        }
    }
}

fn listed_version(expanded: &Map<String, Value>) -> Option<ListedVersion> {
    Some(ListedVersion {
        version: expanded.get(VERSION)?.as_str()?.to_owned(),
        dist_url: expanded
            .get(DIST)
            .and_then(|dist| dist.get(DIST_URL))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

impl PackageListing {
    pub fn find(&self, wanted: &str) -> Option<&ListedVersion> {
        self.versions
            .iter()
            .find(|listed| listed.version == wanted)
            .or_else(|| {
                self.versions
                    .iter()
                    .find(|listed| without_leading_v(&listed.version) == without_leading_v(wanted))
            })
    }
}

fn without_leading_v(version: &str) -> &str {
    version.strip_prefix('v').unwrap_or(version)
}
