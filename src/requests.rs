use crate::art::*;
use crate::edit::EditOperation;
use crate::pixels::{Background, Transform};
use crate::presentation::InspectPresentation;
use crate::selection::CreateSelection;
use crate::workbench_protocol::*;
use crate::workflow::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Initial {
    Fill {
        index: u16,
    },
    /// Exact index data supplied by the caller; do not transcribe an existing image into rows. Use prepare_image for PNG files.
    Indices {
        rows: Vec<Vec<u16>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateArt {
    pub target: Option<Target>,
    pub initial: Option<Initial>,
    pub bundle_path: Option<String>,
    #[serde(default)]
    pub context: ArtContext,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListArt {
    pub resource_id: Option<String>,
    pub group_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectArt {
    pub art_id: String,
    pub region: Option<Rect>,
    #[serde(default)]
    pub include_indices: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtId {
    pub art_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArt {
    pub request_id: Option<String>,
    pub art_id: String,
    pub write_region: Rect,
    #[schemars(length(min = 1, max = 256))]
    pub operations: Vec<EditOperation>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditArtSet {
    pub request_bindings: Option<Vec<RequestBinding>>,
    pub template_art_id: String,
    #[schemars(length(min = 1, max = 16))]
    pub art_ids: Vec<String>,
    pub write_region: Rect,
    pub operations: Vec<EditOperation>,
    pub preview: bool,
    pub expected_plan_hash: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareImage {
    /// PNG path relative to the server workspace, not the agent working directory; or a stored source:<sha256> handle. External files must first be copied into the workspace.
    pub source_path: String,
    /// Required existing art defining the target. Create a fill canvas first if needed; use the returned art_id.
    pub target_art_id: String,
    pub transform: Transform,
    pub provenance: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachReference {
    pub art_id: String,
    /// PNG path relative to the server workspace, not the agent working directory; or a stored source:<sha256> handle. External files must first be copied into the workspace.
    pub source_path: String,
    pub label: String,
    pub role: ReferenceRole,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderArt {
    pub art_id: String,
    pub region: Option<Rect>,
    pub scale: Option<u32>,
    pub background: Option<Background>,
    #[serde(default)]
    pub grid: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareArt {
    pub before_art_id: String,
    pub after_art_id: String,
    pub region: Option<Rect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FocusArt {
    pub reference: Option<ReferenceFocus>,
    pub art_id: String,
    pub region: Rect,
    pub context_padding: u32,
    #[schemars(range(min = 1, max = 64))]
    pub scale: u32,
    #[serde(default)]
    pub grid: bool,
    #[serde(default)]
    pub include_indices: bool,
    pub compare_to_art_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_selection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_selection_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preserve_notes: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub palette_roles: Vec<PaletteRole>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_regions: Vec<ReferenceRegion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_art_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_context: Option<FrameContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_review_id: Option<String>,
    pub base_art_id: String,
    pub write_region: Rect,
    pub instruction: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEditRequests {
    pub status: Option<String>,
    pub cursor: Option<i64>,
    pub limit: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitEditResult {
    pub request_id: String,
    pub result_art_id: String,
    pub notes: String,
}

pub fn input_schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("JSON Schema is serializable")
}
pub fn tool_schemas() -> Vec<(&'static str, &'static str, bool, Value)> {
    vec![
        (
            "open_workbench",
            "Open a managed human workbench with an explicit control_id (fresh random ID per task, 16..128 ASCII letters/digits/_/-). Reuse the same ID across calls or MCP connections. Other controllers and the shared limit prevent launch. Optional idle_timeout_seconds: 1..1800; default 1800. Reopening renews activity without changing the existing timeout.",
            false,
            input_schema::<OpenWorkbench>(),
        ),
        (
            "inspect_workbench",
            "Inspect using the task's explicit control_id, without opening or renewing idle time. owned includes the exact instance and URL; busy means a different control_id owns the screen; closing means HTTP is draining; closed means no active host.",
            true,
            input_schema::<InspectWorkbench>(),
        ),
        (
            "close_workbench",
            "Close using the explicit control_id and exact workbench_id, from any MCP connection to this workspace. Wait for closed; if closing is returned, inspect again. Art and drafts remain. An old ID cannot close a newer instance.",
            false,
            input_schema::<CloseWorkbench>(),
        ),
        (
            "present_art",
            "Present art through a managed workbench. Pass its control_id and the exact workbench_id from open_workbench and a view containing title, items and the expected presentation/state IDs from inspect_presentation. Both expected IDs must be explicit null for the first view. Humans paint, mark issues, undo once and save.",
            false,
            input_schema::<ManagedPresentArt>(),
        ),
        (
            "inspect_presentation",
            "Read the current or archived presentation, human drafts and explicitly saved candidates. state.concerns and saved.concerns contain item_index, art_id, bounds and exact pixels marked by the human; absent means empty. Marks indicate observation, not edit permission or rejection. Use state_id for past states; prepare history and filters yourself with present_art.",
            true,
            input_schema::<InspectPresentation>(),
        ),
        (
            "review_edit_result",
            "Record only acceptance, requested changes, rejection and region feedback explicitly expressed by the user. Put their actual reasoning in notes. Saving or passing validation is not acceptance. Supply expected_review_id to detect conflicts.",
            false,
            input_schema::<ReviewEditResult>(),
        ),
        (
            "create_selection",
            "Store an immutable selection of a rectangle, explicit pixels or a connected region of palette indices. region uses full-art coordinates; connectivity is 4 or 8.",
            false,
            input_schema::<CreateSelection>(),
        ),
        (
            "inspect_edit_request",
            "Read the pinned intent, constraints, protected selections, references, submission and reviews. Continue history using review_cursor, follow_up_cursor and limit.",
            true,
            input_schema::<InspectEditRequest>(),
        ),
        (
            "render_art_set",
            "Compare explicit candidates in the supplied order with optional anchors. Use sheet or frames, scale 1..64 and at most 16 frames. Combined image area is limited.",
            true,
            input_schema::<RenderArtSet>(),
        ),
        (
            "request_edit",
            "Store a regional edit instruction with its base art ID. Coordinates refer to original art pixels.",
            false,
            input_schema::<RequestEdit>(),
        ),
        (
            "list_edit_requests",
            "List stored regional edit requests and submitted results. Filter by pending or submitted status.",
            true,
            input_schema::<ListEditRequests>(),
        ),
        (
            "submit_edit_result",
            "Attach a result candidate to an edit request. It must descend from the base and preserve pixels outside the allowed area. Human acceptance is separate.",
            false,
            input_schema::<SubmitEditResult>(),
        ),
        (
            "list_art",
            "Find stored candidates. limit is 1..100; the response includes resource limits.",
            true,
            input_schema::<ListArt>(),
        ),
        (
            "create_art",
            "Create a fill canvas or exact index data, or import an exported bundle_path. For existing PNGs, create a fill canvas then call prepare_image.",
            false,
            input_schema::<CreateArt>(),
        ),
        (
            "inspect_art",
            "Read target constraints, provenance and references. Index lookup is limited to 4096 pixels.",
            true,
            input_schema::<InspectArt>(),
        ),
        (
            "edit_art",
            "Apply up to 256 ordered operations within the specified region and store a new candidate atomically. Any error leaves the entire action unapplied.",
            false,
            input_schema::<EditArt>(),
        ),
        (
            "prepare_image",
            "Convert a workspace-relative RGB/RGBA PNG using a required target_art_id and explicit settings. Follow error.details to recover from path errors. Maximum input size is 16 MiB.",
            false,
            input_schema::<PrepareImage>(),
        ),
        (
            "attach_reference",
            "Attach a local PNG as an original or reference and store a new candidate. Art pixels are preserved.",
            false,
            input_schema::<AttachReference>(),
        ),
        (
            "edit_art_set",
            "Apply the same edit to at most 16 compatible candidates. First use preview=true, then apply with the returned plan_hash as expected_plan_hash.",
            false,
            input_schema::<EditArtSet>(),
        ),
        (
            "render_art",
            "Return a PNG preview preserving pixel boundaries. Maximum display area is 1048576 pixels.",
            true,
            input_schema::<RenderArt>(),
        ),
        (
            "focus_art",
            "Inspect a local issue with an overview, enlarged context, palette data and optional before/after comparison. region uses full-art coordinates and grants no write permission. views.image_index counts image blocks only. Combined image area is limited to 1048576 pixels; index lookup to 4096 pixels.",
            true,
            input_schema::<FocusArt>(),
        ),
        (
            "compare_art",
            "Compare index changes and visible differences between candidates for the same target.",
            true,
            input_schema::<CompareArt>(),
        ),
        (
            "validate_art",
            "Check supplied constraints and report pass, fail or unknown. Does not assess gameplay or aesthetic acceptance.",
            true,
            input_schema::<ArtId>(),
        ),
        (
            "export_art",
            "Export a candidate that passes every required check, including exact index data, PNG, validation and provenance.",
            false,
            input_schema::<ArtId>(),
        ),
    ]
}
