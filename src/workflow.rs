use crate::{art::*, selection::Point};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaletteRole {
    pub label: String,
    pub indices: Vec<u16>,
    pub source_ref: Option<SourceRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceRegion {
    pub source_hash: String,
    pub source_region: Rect,
    pub target_region: Rect,
    pub description: String,
    pub source_ref: Option<SourceRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub art_id: String,
    pub label: String,
    pub anchor: Option<Point>,
    pub duration_ms: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameContext {
    pub frames: Vec<Frame>,
    pub source_ref: Option<SourceRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestBinding {
    pub art_id: String,
    pub request_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectEditRequest {
    pub request_id: String,
    pub review_cursor: Option<i64>,
    pub follow_up_cursor: Option<i64>,
    pub limit: Option<usize>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Accepted,
    ChangesRequested,
    Rejected,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegionComment {
    pub region: Rect,
    pub comment: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["request_id", "result_art_id", "decision", "notes", "regions", "expected_review_id"]))]
pub struct ReviewEditResult {
    pub request_id: String,
    pub result_art_id: String,
    pub decision: ReviewDecision,
    pub notes: String,
    pub regions: Vec<RegionComment>,
    #[serde(deserialize_with = "required_nullable")]
    pub expected_review_id: Option<String>,
    pub follow_up: Option<crate::requests::RequestEdit>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceFocus {
    pub source_hash: String,
    pub region: Rect,
    pub scale: u32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetView {
    Sheet,
    Frames,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderArtSet {
    pub frames: Vec<Frame>,
    pub scale: u32,
    pub view: SetView,
}
