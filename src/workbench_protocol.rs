use crate::presentation::PresentArt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_WORKBENCHES: usize = 4;
pub const DEFAULT_IDLE_SECONDS: u64 = 1800;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenWorkbench {
    pub control_id: String,
    pub idle_timeout_seconds: Option<u64>,
    pub work_state: Option<WorkState>,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectWorkbench {
    pub control_id: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseWorkbench {
    pub control_id: String,
    pub workbench_id: String,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedPresentArt {
    pub control_id: String,
    pub workbench_id: String,
    pub view: PresentArt,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchInstance {
    pub workbench_id: String,
    pub url: String,
    pub idle_timeout_seconds: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkState {
    Working,
    Waiting,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchState {
    Owned,
    Busy,
    Closing,
    Closed,
}
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchStatus {
    pub state: WorkbenchState,
    pub instance: Option<WorkbenchInstance>,
    pub max_workbenches: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_state: Option<WorkState>,
}
