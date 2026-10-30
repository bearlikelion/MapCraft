//! The editing session: undo, selection, edit operations and the command registry.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod boolean;
mod commands;
pub mod control;
mod curves;
mod generate;
mod issues;
mod link;
mod mirror;
mod noise;
mod ops;
mod organize;
mod prefab;
mod session;
mod terrain;
mod uv;
mod vertex;

pub use commands::{CommandSpec, commands, execute, find};
pub use generate::generator_transform;
pub use issues::{Fix, Issue, LINK_KEYS, Severity, is_broken_link, is_unresolved, target_names, validate};
pub use noise::{FractalType, HeightNoise, NoiseType};
pub use ops::{ClipKeep, ExtrudeMode, Push, Randomize, Shape};
pub use organize::{entities_of, find_entities};
pub use prefab::{expand_instance, flatten, flatten_for_game, prefab_transform};
pub use session::{SelectMode, Session, pick_target};
pub use terrain::{BlendFill, Falloff, Sculpt, SculptMode};
pub use uv::{FacePick, FaceTransfer, face_polygons, uv_islands};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Edit(#[from] mapcraft_map::EditError),
    #[error(transparent)]
    Brush(#[from] mapcraft_map::BrushError),
    #[error(transparent)]
    Format(#[from] mapcraft_format::FormatError),
    #[error("nothing is selected")]
    NothingSelected,
    #[error("the map has not been saved yet")]
    NoPath,
    #[error("bad argument: {0}")]
    Argument(String),
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("`{0}` is not available right now")]
    Disabled(&'static str),
    #[error("refused: {0}")]
    Refused(&'static str),
    #[error("prefab: {0}")]
    Prefab(String),
    #[error("internal error in `{0}`; the map was kept")]
    Panic(&'static str),
}
