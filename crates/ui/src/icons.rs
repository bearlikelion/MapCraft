//! The editor's SVG icons, from @icons by Voxybuns and Godot's editor, rasterized white and tinted per state.

use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

use craft_ui::icons::{SvgIconSet, SvgStyle};
use egui::{Atom, Color32, Image, Rect, Vec2};
use mapcraft_map::NodeKind;

use crate::chrome::{MIRROR, TEXT, TEXT_WEAK};

/// Icons are drawn this many points square, like Godot's editor icons.
pub(crate) const SIZE: f32 = 16.0;

const BRUSH: Color32 = Color32::from_rgb(0xfc, 0x7f, 0x7f);
const ENTITY: Color32 = Color32::from_rgb(0x8e, 0xef, 0x97);
const PREFAB: Color32 = Color32::from_rgb(0x8d, 0xa5, 0xf3);
pub(crate) const GENERATOR: Color32 = Color32::from_rgb(0xff, 0xca, 0x5f);

macro_rules! catalog {
    ($($dir:literal: [$($name:literal),* $(,)?]),* $(,)?) => {
        &[$($(($name, include_bytes!(concat!("../assets/icons/", $dir, "/", $name, ".svg")) as &[u8]),)*)*]
    };
}

static FILES: &[(&str, &[u8])] = catalog! {
    "at-icons": [
        "angle", "arrow_clockwise", "arrow_counterclockwise", "arrow_cross", "arrow_double_vertical", "arrow_down", "arrow_down_to_line", "arrow_left", "arrow_left_to_line", "arrow_merge", "arrow_right", "arrow_right_arrow_left", "arrow_right_to_line", "arrow_up", "arrow_up_from_line", "arrow_up_to_line", "arrow_uturn_up_left", "arrow_uturn_up_right", "arrows_clockwise", "box", "box_wireframe", "brush", "bucket_fill", "bullseye", "checkerboard", "checkmark_in_square", "circle_half_filled", "clipboard", "clock", "cloud_arrow_down", "cog", "cross_in_square", "cube", "cursor", "cylinder", "danger", "dash_in_square", "delivery_box", "diamond", "dice", "doorway", "doorway_exit", "duplicate", "edge", "eraser", "expand", "eye", "eye_closed", "face", "fast_forward", "file_arrow_down", "file_arrow_up", "file_cog", "file_plus", "files", "flag_checkered", "floor_plane", "floppy_disk", "floppy_disks", "folder", "folder_open", "footsteps", "gimbal", "globe", "grid_coarse", "grid_fine", "help", "hexagon", "hourglass", "house", "icons", "itinerary", "jigsaw_piece", "keyboard", "knife", "layers", "link", "list_checkboxes", "list_unordered", "location", "lock", "magic_wand", "magnet", "magnifying_glass", "mesh_polygon", "mesh_quad", "mirror_horizontally", "mirror_vertically", "mountains", "no_entry", "node_graph_root", "orbit", "palette", "paperclip", "pencil", "play", "plus", "plus_sign_in_square", "prism", "projector", "pyramid", "recover", "resize", "reticle", "road", "rotate", "scissors", "selection_square", "sky_dome", "sliders", "sphere", "square", "storage", "sun", "swatches", "tag", "thumbtack", "tileset", "trash_can", "unlock", "unwrap", "vertex", "wave", "window", "window_terminal", "windows", "wrench",
    ],
    "godot": ["ActionCopy", "CenterView", "CreateNewSceneFrom", "Group", "Instance", "PackedScene", "SnapGrid", "Ungroup", "Unlinked"],
};

fn set() -> &'static SvgIconSet {
    static ICONS: OnceLock<SvgIconSet> = OnceLock::new();
    ICONS.get_or_init(|| SvgIconSet::new(FILES, SvgStyle { current_color: "white", stroke_width: None }, "bytes://mapcraft/icons/", "help"))
}

/// Icon `name`, white at 16 points; `.tint()` gives it its final color.
pub(crate) fn icon(name: &str) -> Image<'static> {
    if !set().contains(name) {
        static WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
        if let Ok(mut warned) = WARNED.lock()
            && warned.insert(name.to_string())
        {
            log::warn!("icon `{name}` is not vendored; showing help instead");
        }
    }
    Image::new(set().source(name)).fit_to_exact_size(Vec2::splat(SIZE))
}

/// Paints icon `name` centered in `rect`, `size` points square.
pub(crate) fn paint(ui: &egui::Ui, name: &str, rect: Rect, size: f32, tint: Color32) {
    icon(name).tint(tint).paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(size)));
}

/// The icon column of a menu item or button: icon `name`, or an empty slot so labels line up.
pub(crate) fn slot(name: Option<&str>) -> Atom<'static> {
    match name {
        Some(n) => icon(n).tint(TEXT).into(),
        None => Atom { size: Some(Vec2::splat(SIZE)), ..Default::default() },
    }
}

/// A button with icon `name` before `text`; the icon follows the text color through hover and disabled.
pub(crate) fn button<'a>(name: Option<&str>, text: impl Into<egui::WidgetText>) -> egui::Button<'a> {
    egui::Button::new((slot(name), Atom::from(text.into()))).image_tint_follows_text_color(true)
}

/// A selectable button or menu item with icon `name` before `text`.
pub(crate) fn selectable<'a>(on: bool, name: Option<&str>, text: impl Into<egui::WidgetText>) -> egui::Button<'a> {
    egui::Button::selectable(on, (slot(name), Atom::from(text.into()))).image_tint_follows_text_color(true)
}

/// The icon shown for an action or command id in menus, toolbars and the right click menu.
pub(crate) fn action_icon(id: &str) -> Option<&'static str> {
    Some(match id {
        "tool.select" => "cursor",
        "tool.move" => "arrow_cross",
        "tool.extrude" => "floor_plane",
        "tool.rotate" => "rotate",
        "tool.scale" => "resize",
        "tool.block" => "box",
        "tool.entity" => "location",
        "tool.clip" => "scissors",
        "tool.vertex" | "vertex.mode_vertex" => "vertex",
        "tool.face" | "vertex.mode_face" => "face",
        "tool.sculpt" => "mountains",
        "tool.path" => "itinerary",
        "vertex.mode_edge" => "edge",
        "vertex.merge" | "tools.merge" => "arrow_merge",
        "edit.undo" => "arrow_uturn_up_left",
        "edit.redo" => "arrow_uturn_up_right",
        "edit.delete" => "trash_can",
        "edit.duplicate" => "duplicate",
        "edit.duplicate_linked" | "view.link_zoom" => "link",
        "tools.edit_link_here" => "pencil",
        "tools.make_link_real" | "tools.mirror_make_real" | "tools.curve_make_real" | "prefab.make_local" => "Unlinked",
        "edit.select_all" => "checkmark_in_square",
        "edit.select_faces_by_material" => "face",
        "edit.select_brushes_by_material" => "cube",
        "edit.search" => "magnifying_glass",
        "tools.drop_to_floor" => "arrow_down_to_line",
        "tools.randomize" => "dice",
        "file.revert" => "recover",
        "file.reload_assets" => "arrows_clockwise",
        "view.slope" => "angle",
        "run.load_pointfile" => "footsteps",
        "run.unload_pointfile" => "cross_in_square",
        "run.point_next" => "arrow_right",
        "run.point_prev" => "arrow_left",
        "edit.select_none" => "cross_in_square",
        "edit.select_groups" | "tools.group" => "Group",
        "edit.select_objects" => "icons",
        "edit.select_solids" | "uv.cube_projection" => "cube",
        "tools.ungroup" => "Ungroup",
        "tools.carve" => "knife",
        "tools.zone_from_selection" => "flag_checkered",
        "tools.snap_to_grid" => "SnapGrid",
        "tools.mirror" | "tools.flip_x" | "tools.flip_y" => "mirror_horizontally",
        "tools.flip_z" => "mirror_vertically",
        "tools.sweep" => "road",
        "tools.spin" => "orbit",
        "tools.array_path" | "tools.array" => "files",
        "tools.tie_to_entity" => "paperclip",
        "tools.move_to_world" => "globe",
        "tools.center_origins" | "vertex.origin_to_corners" => "reticle",
        "vertex.delete" => "trash_can",
        "clip.remove_point" => "dash_in_square",
        "tools.rotate_cw" => "arrow_clockwise",
        "tools.rotate_ccw" => "arrow_counterclockwise",
        "tools.apply_material" => "bucket_fill",
        "tools.clear_material" => "eraser",
        "tools.toggle_texture_lock" | "view.lock_selected" => "lock",
        "view.unlock_all" => "unlock",
        "view.hide_selected" => "eye_closed",
        "view.show_all" => "eye",
        "view.grid_smaller" => "grid_fine",
        "view.grid_larger" => "grid_coarse",
        "uv.unwrap" => "unwrap",
        "uv.project_from_view" => "projector",
        "uv.reset" => "recover",
        "uv.fit" => "bullseye",
        "file.new" => "file_plus",
        "file.open" => "folder_open",
        "file.save" => "floppy_disk",
        "file.save_as" => "floppy_disks",
        "file.switch_sdk" => "arrow_right_arrow_left",
        "file.sdk_settings" => "file_cog",
        "file.sdk_manager" => "delivery_box",
        "file.quit" => "doorway_exit",
        "edit.preferences" => "cog",
        "tools.check_problems" => "list_checkboxes",
        "tools.transform" => "gimbal",
        "tools.replace_materials" | "panel.materials" => "swatches",
        "prefab.save" => "CreateNewSceneFrom",
        "prefab.insert" => "Instance",
        "prefab.reload" => "arrows_clockwise",
        "layer.new" => "plus",
        "view.render_textured" => "checkerboard",
        "view.render_flat" => "circle_half_filled",
        "view.render_wireframe" => "box_wireframe",
        "view.render_lit" => "sun",
        "view.sky" => "sky_dome",
        "view.colorize" => "palette",
        "view.snap_grid" => "magnet",
        "view.snap_vertices" => "mesh_quad",
        "view.frame_all" => "CenterView",
        "view.frame_selection" => "bullseye",
        "view.origin" => "house",
        "view.maximize" => "expand",
        "view.single" => "window",
        "view.find_entities" | "panel.scene_search" => "magnifying_glass",
        "panel.scene" => "node_graph_root",
        "panel.tags" => "tag",
        "panel.entities" => "diamond",
        "panel.models" => "mesh_polygon",
        "panel.output" => "window_terminal",
        "run.again" => "fast_forward",
        "run.cancel" => "square",
        "help.shortcuts" => "keyboard",
        _ => return None,
    })
}

/// The icon of a selection mode, the same as its Edit menu command's.
pub(crate) fn select_mode_icon(mode: mapcraft_engine::SelectMode) -> &'static str {
    use mapcraft_engine::SelectMode as M;
    let id = match mode {
        M::Groups => "edit.select_groups",
        M::Objects => "edit.select_objects",
        M::Solids => "edit.select_solids",
    };
    action_icon(id).unwrap_or("help")
}

/// The icon and color a node kind shows in the scene tree, like Godot's class icons.
pub(crate) fn node_icon(kind: &NodeKind, entity_color: Option<Color32>) -> (&'static str, Color32) {
    match kind {
        NodeKind::World(_) => ("globe", TEXT_WEAK),
        NodeKind::Layer(_) => ("layers", TEXT_WEAK),
        NodeKind::Group(g) if g.generator.is_some() => ("magic_wand", GENERATOR),
        NodeKind::Group(g) if g.curve.is_some() => ("itinerary", crate::curves::PATH_COLOR),
        NodeKind::Group(g) if g.mirror.is_some() => ("mirror_horizontally", MIRROR),
        NodeKind::Group(_) => ("folder", TEXT_WEAK),
        NodeKind::Entity(_) => ("diamond", entity_color.unwrap_or(ENTITY)),
        NodeKind::Brush(_) => ("cube", BRUSH),
        NodeKind::Prefab(_) => ("PackedScene", PREFAB),
        NodeKind::Path(_) => ("itinerary", crate::curves::PATH_COLOR),
    }
}

/// The icon and color of an entity class in the Entities panel.
pub(crate) fn class_icon(solid: bool, color: Option<Color32>) -> (&'static str, Color32) {
    if solid { ("cube", color.unwrap_or(BRUSH)) } else { ("diamond", color.unwrap_or(ENTITY)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_name_used_is_in_the_catalog() {
        use mapcraft_map::{Brush, Entity, Generator, Group, Layer, Path, Prefab};
        let generator = Generator {
            plugin: String::new(),
            command: String::new(),
            params: serde_json::Value::Null,
            origin: Default::default(),
            angles: Default::default(),
        };
        let kinds = [
            NodeKind::World(Entity::default()),
            NodeKind::Layer(Layer::default()),
            NodeKind::Group(Group::default()),
            NodeKind::Group(Group { generator: Some(generator), ..Group::default() }),
            NodeKind::Entity(Entity::default()),
            NodeKind::Brush(Brush::default()),
            NodeKind::Prefab(Prefab::default()),
            NodeKind::Path(Path::default()),
        ];
        let names = crate::shortcuts::actions()
            .filter_map(|(id, _)| action_icon(id))
            .chain(crate::chrome::block_shapes().map(|(_, _, icon)| icon))
            .chain(kinds.iter().map(|k| node_icon(k, None).0))
            .chain([class_icon(true, None).0, class_icon(false, None).0]);
        for name in names {
            assert!(set().contains(name), "{name} is not vendored");
        }
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        for (name, _) in FILES {
            let egui::ImageSource::Bytes { uri, bytes } = set().source(name) else { panic!("{name} is not embedded") };
            ctx.include_bytes(uri.clone(), bytes);
            let hint = egui::load::SizeHint::Size { width: 32, height: 32, maintain_aspect_ratio: true };
            assert!(matches!(ctx.try_load_image(&uri, hint), Ok(egui::load::ImagePoll::Ready { .. })), "{name} does not rasterize");
        }
    }
}
