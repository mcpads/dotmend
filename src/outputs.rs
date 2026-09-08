use crate::{
    art::*,
    presentation::PresentedArt,
    requests::{RequestEdit, input_schema},
    selection::Point,
    validation::Validation,
    workbench_protocol::WorkbenchStatus,
    workflow::*,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

macro_rules! object {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug,Clone,Serialize,Deserialize,JsonSchema)]
        #[serde(deny_unknown_fields)]
        #[schemars(extend("required" = [$(stringify!($field)),*]))]
        pub struct $name { $(pub $field:$ty),* }
    };
}
object!(ArtSummary {art_id:String,resource_id:String,width:u32,height:u32,parents:Vec<String>,context:ArtContext,origin:Option<String>});
object!(Limits {
    max_dimension: u32,
    max_pixels: usize,
    max_palette: usize,
    max_operations: usize,
    max_edit_writes: usize,
    max_set_size: usize,
    max_preview_pixels: usize,
    max_indices_reply: usize,
    max_concern_pixels: usize,
    max_source_bytes: usize,
    max_source_pixels: usize
});
object!(CreatedArt {
    art_id: String,
    summary: ArtSummary,
    provenance: Value
});
object!(ArtList {arts:Vec<ArtSummary>,next_cursor:Option<String>,limits:Limits});
object!(InspectedArt {art_id:String,target:Target,context:ArtContext,references:Vec<Reference>,parents:Vec<String>,provenance:Value,usage:Vec<usize>,region:Rect,indices:Option<Vec<Vec<u16>>>});
object!(EditedArt {art_id:String,base_art_id:String,changed_pixels:usize,changed_region:Option<Rect>});
object!(RenderedArt {
    art_id: String,
    region: Rect,
    scale: u32,
    display_width: u32,
    display_height: u32
});
object!(ComparedArt {before_art_id:String,after_art_id:String,region:Rect,changed_pixels:usize,visual_changed_pixels:usize,changed_region:Option<Rect>});
object!(PreparedArt {
    art_id: String,
    diagnostics: ConversionDiagnostics,
    provenance: Value
});
object!(ConversionDiagnostics {
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    transform: crate::pixels::Transform,
    transparent_pixels: u64,
    tie_pixels: u64,
    mean_squared_rgb_distance: f64,
    max_squared_rgb_distance: u32,
    aspect_ratio_changed: bool
});
object!(AttachedReference {art_id:String,references:Vec<Reference>});
object!(ExportedArt {art_id:String,bundle_id:String,bundle_path:String,files:Vec<String>,validation:Validation});
object!(SelectionSummary {
    selection_id: String,
    base_art_id: String,
    target_hash: String,
    region: Rect,
    bounds: Rect,
    pixel_count: usize,
    mask_uri: String
});
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SetCandidate {
    Compatible {
        art_id: String,
        base_art_id: String,
        changed_pixels: usize,
        changed_region: Option<Rect>,
        validation: Validation,
        image_index: usize,
    },
    Incompatible {
        base_art_id: String,
        error: ArtError,
    },
}
object!(EditedSet {preview:bool,compatible:bool,plan_hash:Option<String>,results:Vec<SetCandidate>,candidates_saved:bool});
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Submitted,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HumanReview {
    Pending,
    Accepted,
    ChangesRequested,
    Rejected,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RequestPayload {
    #[serde(flatten)]
    pub input: RequestEdit,
    pub target_hash: String,
    pub coordinate_system: String,
}
object!(RequestedEdit {
    request_id: String,
    request: RequestPayload
});
object!(RequestEntry {request_id:String,request:RequestPayload,result_art_id:Option<String>,notes:Option<String>,status:DeliveryStatus,human_review:HumanReview,current_review_id:Option<String>});
object!(RequestList {requests:Vec<RequestEntry>,next_cursor:Option<i64>});
object!(ReviewRecord {review_id:String,input:ReviewEditResult,follow_up_request_id:Option<String>});
object!(InspectedRequest {request_id:String,request:RequestPayload,target:Target,previous_request:Option<RequestEntry>,previous_review:Option<ReviewRecord>,selections:Vec<SelectionSummary>,references:Vec<Reference>,related_arts:Vec<ArtSummary>,result_art_id:Option<String>,notes:Option<String>,status:DeliveryStatus,human_review:HumanReview,current_review_id:Option<String>,latest_review:Option<ReviewRecord>,reviews:Vec<ReviewRecord>,next_review_cursor:Option<i64>,follow_up_requests:Vec<RequestEntry>,next_follow_up_cursor:Option<i64>});
object!(SubmittedEdit {request_id:String,result_art_id:String,status:DeliveryStatus,human_review:HumanReview,current_review_id:Option<String>,diff:EditedArt});
object!(SavedReview {review:ReviewRecord,current_review_id:Option<String>,human_review:HumanReview});
object!(ClippedContext {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32
});
object!(Overlay {
    role: String,
    region: Rect,
    rgb: String
});
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FocusView {
    pub role: String,
    pub art_id: Option<String>,
    pub before_art_id: Option<String>,
    pub source_hash: Option<String>,
    pub coordinate_system: String,
    pub region: Rect,
    pub scale: u32,
    pub grid: bool,
    pub overlays: Option<Vec<Overlay>>,
    pub image_index: usize,
    pub display_width: u32,
    pub display_height: u32,
}
object!(FocusComparison {
    focus: ComparedArt,
    context: ComparedArt
});
object!(FocusedArt {art_id:String,target_hash:String,focus_region:Rect,context_region:Rect,context_padding:u32,context_clipped:ClippedContext,coordinate_system:String,scale:u32,views:Vec<FocusView>,display_pixels:u64,palette:Vec<String>,transparent_index:Option<u16>,allowed_indices:Vec<u16>,focus_usage:Vec<usize>,indices:Option<Vec<Vec<u16>>>,comparison:Option<FocusComparison>,references:Vec<Reference>,implementation:String});
object!(FrameView {art_id:String,label:String,image_index:usize,region:Rect,display_region:Rect,scale:u32,anchor:Option<Point>,duration_ms:Option<u32>,coordinate_system:String});
object!(RenderedSet {frames:Vec<FrameView>,view:SetView,scale:u32,display_pixels:u64,display_width:u32,display_height:u32,alignment:String,playable:bool,implementation:String});

fn schema<T: JsonSchema>() -> Value {
    let mut success = input_schema::<T>();
    success
        .as_object_mut()
        .expect("object schema")
        .remove("$schema");
    let definitions = success
        .as_object_mut()
        .expect("object schema")
        .remove("$defs");
    success["properties"]["ok"] = json!({"const":true});
    success["required"]
        .as_array_mut()
        .expect("required fields")
        .push(json!("ok"));
    let mut error = input_schema::<ArtError>();
    error
        .as_object_mut()
        .expect("error object")
        .remove("$schema");
    let mut result = json!({"type":"object","oneOf":[success,{"type":"object","properties":{"ok":{"const":false},"error":error},"required":["ok","error"],"additionalProperties":false}]});
    if let Some(defs) = definitions {
        result["$defs"] = defs;
    }
    result
}
fn check<T: serde::de::DeserializeOwned>(data: &Value) -> ArtResult<()> {
    let mut value = data.clone();
    if value["ok"] != true {
        return Err(ArtError::new(
            "internal_error",
            "Success response must contain ok=true",
        ));
    }
    value.as_object_mut().expect("success object").remove("ok");
    serde_json::from_value::<T>(value).map(|_| ()).map_err(|e| {
        ArtError::new(
            "internal_error",
            format!("Invalid success response format: {e}"),
        )
    })
}
macro_rules! output_types {
    ($callback:ident,$name:expr $(,$data:expr)?) => {
        match $name {
            "open_workbench"|"inspect_workbench"|"close_workbench"=>$callback::<WorkbenchStatus>($($data)?),
            "present_art"|"inspect_presentation"=>$callback::<PresentedArt>($($data)?),
            "review_edit_result"=>$callback::<SavedReview>($($data)?),
            "create_art"=>$callback::<CreatedArt>($($data)?),"list_art"=>$callback::<ArtList>($($data)?),
            "inspect_art"=>$callback::<InspectedArt>($($data)?),"edit_art"=>$callback::<EditedArt>($($data)?),
            "render_art"=>$callback::<RenderedArt>($($data)?),"compare_art"=>$callback::<ComparedArt>($($data)?),
            "prepare_image"=>$callback::<PreparedArt>($($data)?),"attach_reference"=>$callback::<AttachedReference>($($data)?),
            "export_art"=>$callback::<ExportedArt>($($data)?),"validate_art"=>$callback::<Validation>($($data)?),
            "create_selection"=>$callback::<SelectionSummary>($($data)?),"edit_art_set"=>$callback::<EditedSet>($($data)?),
            "request_edit"=>$callback::<RequestedEdit>($($data)?),"list_edit_requests"=>$callback::<RequestList>($($data)?),
            "inspect_edit_request"=>$callback::<InspectedRequest>($($data)?),"submit_edit_result"=>$callback::<SubmittedEdit>($($data)?),
            "focus_art"=>$callback::<FocusedArt>($($data)?),"render_art_set"=>$callback::<RenderedSet>($($data)?),
            _=>unreachable!("registered tool output"),
        }
    }
}
pub fn output_schema(name: &str) -> Value {
    output_types!(schema, name)
}
pub fn check_output(name: &str, data: &Value) -> ArtResult<()> {
    output_types!(check, name, data)
}
