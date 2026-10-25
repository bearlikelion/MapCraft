use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub recent: Vec<String>,
    pub last_sdk: Option<String>,
    /// Extra folders searched for SDKs.
    pub sdk_dirs: Vec<String>,
    /// Per SDK id, the folders and programs the user picked for its `paths`.
    pub sdk_paths: BTreeMap<String, BTreeMap<String, String>>,
    /// SDKs whose compile tools the user allowed to run.
    pub approved_tools: BTreeSet<String>,
    pub view: ViewPrefs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewPrefs {
    pub grid: f64,
    pub autosave_minutes: u32,
    pub fly_speed: f64,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self { grid: 16.0, autosave_minutes: 5, fly_speed: 512.0 }
    }
}

impl craft_settings::Settings for Prefs {
    const APP_ID: &'static str = "mapcraft";
    const SCHEMA_VERSION: u32 = 1;
}

impl Prefs {
    pub fn push_recent(&mut self, path: &str) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_string());
        self.recent.truncate(10);
    }
}
