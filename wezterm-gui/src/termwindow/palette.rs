use config::keyassignment::KeyAssignment;
use frecency::Frecency;
use serde::{Deserialize, Serialize};
use wezterm_dynamic::{FromDynamic, ToDynamic};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Recent {
    brief: String,
    frecency: Frecency,
}

#[derive(Debug, Clone, FromDynamic, ToDynamic)]
pub struct UserPaletteEntry {
    pub brief: String,
    pub doc: Option<String>,
    pub action: KeyAssignment,
    pub icon: Option<String>,
}

