use crate::{art::*, edit::Pixel, selection::Point};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_CONCERN_PIXELS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PresentationItem {
    Art {
        art_id: String,
        label: String,
        region: Rect,
        scale: u32,
        editable: bool,
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        playback: Option<Vec<crate::workflow::Frame>>,
    },
    Reference {
        art_id: String,
        source_hash: String,
        label: String,
        region: Rect,
        scale: u32,
    },
}
impl PresentationItem {
    pub fn art_id(&self) -> &str {
        match self {
            Self::Art { art_id, .. } | Self::Reference { art_id, .. } => art_id,
        }
    }
    pub fn region(&self) -> Rect {
        match self {
            Self::Art { region, .. } | Self::Reference { region, .. } => *region,
        }
    }
    pub fn scale(&self) -> u32 {
        match self {
            Self::Art { scale, .. } | Self::Reference { scale, .. } => *scale,
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Self::Art { label, .. } | Self::Reference { label, .. } => label,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["title", "items", "expected_presentation_id", "expected_state_id"]))]
pub struct PresentArt {
    pub title: String,
    #[serde(default)]
    pub note: String,
    pub items: Vec<PresentationItem>,
    #[serde(deserialize_with = "required_nullable")]
    pub expected_presentation_id: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub expected_state_id: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectPresentation {
    pub presentation_id: Option<String>,
    pub state_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresentationActionKind {
    Open,
    Paint,
    Mark,
    Undo,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Concern {
    pub item_index: usize,
    pub art_id: String,
    pub bounds: Rect,
    pub pixels: Vec<Point>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresentationState {
    pub presentation_id: String,
    pub art_ids: Vec<String>,
    pub previous_state_id: Option<String>,
    pub undo_art_ids: Option<Vec<String>>,
    pub action: PresentationActionKind,
    // Omit absent additions so previously stored states retain their hashes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub concerns: Vec<Concern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_concerns: Option<Vec<Concern>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SavedPresentation {
    pub save_id: String,
    pub presentation_id: String,
    pub state_id: String,
    pub art_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub concerns: Vec<Concern>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresentationSnapshot {
    pub is_current: bool,
    pub presentation_id: String,
    pub presentation: PresentArt,
    pub state_id: String,
    pub current_state_id: String,
    pub state: PresentationState,
    pub saved: Option<SavedPresentation>,
    pub dirty: bool,
    pub undo_available: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["presentation"]))]
pub struct PresentedArt {
    pub presentation: Option<PresentationSnapshot>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HumanAction {
    Mark {
        presentation_id: String,
        expected_state_id: String,
        item_index: usize,
        pixels: Vec<Point>,
        marked: bool,
    },
    Paint {
        presentation_id: String,
        expected_state_id: String,
        item_index: usize,
        pixels: Vec<Pixel>,
    },
    Undo {
        presentation_id: String,
        expected_state_id: String,
    },
    Save {
        presentation_id: String,
        expected_state_id: String,
    },
}
impl HumanAction {
    pub fn identity(&self) -> (&str, &str) {
        match self {
            Self::Mark {
                presentation_id,
                expected_state_id,
                ..
            }
            | Self::Paint {
                presentation_id,
                expected_state_id,
                ..
            }
            | Self::Undo {
                presentation_id,
                expected_state_id,
            }
            | Self::Save {
                presentation_id,
                expected_state_id,
            } => (presentation_id, expected_state_id),
        }
    }
}
