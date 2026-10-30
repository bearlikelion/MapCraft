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
    /// Per SDK id, the user's own texture folders, searched after the SDK's.
    pub sdk_textures: BTreeMap<String, Vec<String>>,
    /// Per SDK id, the material folders the Materials panel leaves out, with all in them.
    pub hidden_textures: BTreeMap<String, BTreeSet<String>>,
    /// Per SDK id, the model folders the Models tab leaves out, with all in them.
    pub hidden_models: BTreeMap<String, BTreeSet<String>>,
    /// Per SDK id and material, the Lit maps and values picked over the ones found.
    pub material_lit: BTreeMap<String, BTreeMap<String, crate::project::LitMaps>>,
    /// SDKs whose compile tools the user allowed to run.
    pub approved_tools: BTreeSet<String>,
    pub view: ViewPrefs,
    pub keymap: crate::shortcuts::Preset,
    /// The user's own chords by action id, over the preset; empty unbinds.
    pub shortcuts: BTreeMap<String, String>,
    /// Inspector sections folded open or closed, by `{key}:{closed by default}`.
    pub inspector_folds: BTreeMap<String, bool>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            last_sdk: None,
            sdk_dirs: Vec::new(),
            registries: vec![mapcraft_sdk_manager::DEFAULT_REGISTRY.to_string()],
            sdk_paths: BTreeMap::new(),
            sdk_textures: BTreeMap::new(),
            hidden_textures: BTreeMap::new(),
            hidden_models: BTreeMap::new(),
            material_lit: BTreeMap::new(),
            approved_tools: BTreeSet::new(),
            view: ViewPrefs::default(),
            keymap: crate::shortcuts::Preset::default(),
            shortcuts: BTreeMap::new(),
            inspector_folds: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewPrefs {
    pub grid: f64,
    pub autosave_minutes: u32,
    /// Earlier versions of a map kept in a `backups` folder beside it each time it is saved.
    pub backups: u32,
    pub fly_speed: f64,
    /// Draw each brush in its own color in the 2D views, as Hammer does.
    pub colorize: bool,
    pub snap_grid: bool,
    /// Dragged points snap to nearby brush corners and entity origins.
    pub snap_vertices: bool,
    /// Zooming one 2D view zooms them all.
    pub link_zoom: bool,
    /// Clicking a material in the browser applies it to the selected brushes or picked faces.
    pub apply_on_click: bool,
    /// One view fills the window and Space shows the next one, as in TrenchBroom.
    pub single_view: bool,
    /// A face drag sweeps the face out square to it instead of moving its plane.
    pub extrude_along_normal: bool,
    /// Tint faces by steepness: walkable, surfable, wall or ceiling.
    pub slope_colors: bool,
    /// Steepest a floor can be and still be stood on, in degrees; Source's is 45.573 (normal Z 0.7).
    pub slope_walkable: f64,
    /// Steepest a surfable ramp can be, in degrees; surf maps' main ramps stay under about 72.
    pub slope_surfable: f64,
    /// 3D views draw the sky; off, sky faces draw like other faces.
    pub sky: bool,
    /// The corner of the views that shows each shortcut pressed; `None` hides them.
    pub show_keys: Option<crate::keys::Corner>,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            grid: 16.0,
            autosave_minutes: 5,
            backups: 3,
            fly_speed: 512.0,
            colorize: true,
            snap_grid: true,
            snap_vertices: true,
            link_zoom: true,
            apply_on_click: true,
            single_view: false,
            extrude_along_normal: false,
            slope_colors: false,
            slope_walkable: 45.573,
            slope_surfable: 72.0,
            sky: true,
            show_keys: None,
        }
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
