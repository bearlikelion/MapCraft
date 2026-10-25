use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub recent: Vec<String>,
    pub last_sdk: Option<String>,
    /// Extra folders searched for SDKs.
    pub sdk_dirs: Vec<String>,
    /// SDK registry URLs or paths the SDK Manager lists SDKs from.
    pub registries: Vec<String>,
    /// Per SDK id, the folders and programs the user picked for its `paths`.
    pub sdk_paths: BTreeMap<String, BTreeMap<String, String>>,
    /// SDKs whose compile tools the user allowed to run.
    pub approved_tools: BTreeSet<String>,
    pub view: ViewPrefs,
    pub keymap: crate::shortcuts::Preset,
    /// The user's own chords by action id, over the preset; empty unbinds.
    pub shortcuts: BTreeMap<String, String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            last_sdk: None,
            sdk_dirs: Vec::new(),
            registries: vec![mapcraft_sdk_manager::DEFAULT_REGISTRY.to_string()],
            sdk_paths: BTreeMap::new(),
            approved_tools: BTreeSet::new(),
            view: ViewPrefs::default(),
            keymap: crate::shortcuts::Preset::default(),
            shortcuts: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewPrefs {
    pub grid: f64,
    pub autosave_minutes: u32,
    pub fly_speed: f64,
    /// Draw each brush in its own color in the 2D views, as Hammer does.
    pub colorize: bool,
    pub snap_grid: bool,
    /// Dragged points snap to nearby brush corners and entity origins.
    pub snap_vertices: bool,
    /// Zooming one 2D view zooms them all.
    pub link_zoom: bool,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self { grid: 16.0, autosave_minutes: 5, fly_speed: 512.0, colorize: true, snap_grid: true, snap_vertices: true, link_zoom: true }
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
