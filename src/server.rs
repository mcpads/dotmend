use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use dotmend::{
    art::*,
    pixels::{art_raster, encode_png},
    requests::tool_schemas,
    workspace::{ToolOutput, Workspace},
};
use rmcp::{ErrorData, RoleServer, ServerHandler, model::*, service::RequestContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub(crate) type SharedWorkspace = Arc<Mutex<Workspace>>;
pub struct ArtServer {
    pub workspace: SharedWorkspace,
    pub workbench: Arc<tokio::sync::Mutex<crate::workbench::Workbench>>,
    pub invocations: Arc<crate::invocations::Invocations>,
}
fn mcp_tools() -> Vec<Tool> {
    tool_schemas()
        .into_iter()
        .map(|(name, description, readonly, schema)| {
            let mut tool = Tool::new(
                name,
                description,
                schema.as_object().cloned().expect("object schema"),
            );
            let mut annotations = ToolAnnotations::new();
            annotations.read_only_hint = Some(readonly);
            annotations.destructive_hint = Some(false);
            annotations.open_world_hint = Some(false);
            tool.annotations = Some(annotations);
            tool.output_schema = Some(Arc::new(
                dotmend::outputs::output_schema(name)
                    .as_object()
                    .unwrap()
                    .clone(),
            ));
            tool
        })
        .collect()
}
async fn execute(shared: SharedWorkspace, name: String, args: Value) -> ArtResult<ToolOutput> {
    tokio::task::spawn_blocking(move || {
        shared
            .lock()
            .map_err(|_| ArtError::new("storage_error", "Failed to lock the workspace"))?
            .call(&name, args)
    })
    .await
    .map_err(|e| ArtError::new("storage_error", e.to_string()))?
}
impl ServerHandler for ArtServer {
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        std::borrow::Cow::Owned(vec![ProtocolVersion::V_2026_07_28])
    }
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("dotmend", implementation_id()))
        .with_protocol_version(ProtocolVersion::V_2026_07_28)
        .with_instructions(crate::server_instructions::INSTRUCTIONS)
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.and_then(|r| r.cursor).is_some() {
            return Err(ErrorData::invalid_params(
                "No tool-list cursor was issued",
                None,
            ));
        }
        Ok(ListToolsResult {
            tools: mcp_tools(),
            ttl_ms: Some(0),
            cache_scope: Some(CacheScope::Private),
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !tool_schemas().iter().any(|t| t.0 == request.name) {
            return Err(ErrorData::invalid_params("Unknown tool", None));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let name = request.name.to_string();
        let result = self.managed_call(&name, arguments).await;
        let result = result.and_then(|output| {
            dotmend::outputs::check_output(&name, &output.data)?;
            Ok(output)
        });
        if let Err(error) = &result
            && error.code == "internal_error"
        {
            return Err(ErrorData::internal_error(error.to_string(), None));
        }
        let mut output = match result {
            Ok(output) => {
                let mut content = vec![ContentBlock::text(output.data.to_string())];
                for image in output.images {
                    content.push(ContentBlock::image(STANDARD.encode(image), "image/png"));
                }
                for uri in output.links {
                    let name = uri.rsplit('/').next().unwrap_or("artifact").to_owned();
                    content.push(ContentBlock::resource_link(Resource::new(uri, name)));
                }
                let mut result = CallToolResult::success(content);
                result.structured_content = Some(output.data);
                result
            }
            Err(error) => {
                let data = json!({"ok":false,"error":error});
                let mut result = CallToolResult::error(vec![ContentBlock::text(data.to_string())]);
                result.structured_content = Some(data);
                result
            }
        };
        output.is_error.get_or_insert(false);
        Ok(output.into())
    }
    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        if request.and_then(|r| r.cursor).is_some() {
            return Err(ErrorData::invalid_params(
                "No resource-list cursor was issued",
                None,
            ));
        }
        Ok(ListResourcesResult {
            resources: vec![Resource::new(
                "dotmend://guides/editing",
                "Editing art through MCP",
            )],
            ttl_ms: Some(0),
            cache_scope: Some(CacheScope::Private),
            ..Default::default()
        })
    }
    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        if request.and_then(|r| r.cursor).is_some() {
            return Err(ErrorData::invalid_params(
                "No resource-template cursor was issued",
                None,
            ));
        }
        Ok(ListResourceTemplatesResult {
            ttl_ms: Some(0),
            cache_scope: Some(CacheScope::Private),
            ..Default::default()
        })
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let (bytes, mime) = self
            .workspace
            .lock()
            .map_err(|_| ErrorData::internal_error("Failed to lock storage", None))?
            .resource(&request.uri)
            .map_err(|e| match e.code.as_str() {
                "source_unavailable" | "selection_not_found" | "resource_not_found" => {
                    ErrorData::resource_not_found(e.to_string(), Some(json!({"uri":request.uri})))
                }
                "invalid_input" => {
                    ErrorData::invalid_params(e.to_string(), Some(json!({"uri":request.uri})))
                }
                _ => ErrorData::internal_error(e.to_string(), Some(json!({"uri":request.uri}))),
            })?;
        let contents = if mime == "application/json" || mime.starts_with("text/") {
            ResourceContents::text(
                String::from_utf8(bytes)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
                &request.uri,
            )
            .with_mime_type(mime)
        } else {
            ResourceContents::blob(STANDARD.encode(bytes), &request.uri).with_mime_type(mime)
        };
        Ok(ReadResourceResult::new(vec![contents])
            .with_ttl_ms(0)
            .with_cache_scope(CacheScope::Private)
            .into())
    }
}

#[derive(Clone)]
struct WebState {
    workspace: SharedWorkspace,
    port: u16,
    access: Arc<crate::workbench::WorkbenchAccess>,
}
#[derive(Deserialize)]
struct ToolCall {
    tool: String,
    #[serde(default)]
    arguments: Value,
}
fn failure(error: ArtError) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"ok":false,"error":error})),
    )
        .into_response()
}
async fn tool_call(State(state): State<WebState>, Json(request): Json<ToolCall>) -> Response {
    if !tool_schemas().iter().any(|t| t.0 == request.tool && t.2)
        || request.tool == "inspect_workbench"
    {
        return failure(ArtError::new(
            "mcp_required",
            "Use the owning MCP connection for mutations and lifecycle tools",
        ));
    }
    match execute(state.workspace, request.tool, request.arguments).await {
        Ok(output) => {
            let mut data = output.data;
            data["images"] = json!(
                output
                    .images
                    .into_iter()
                    .map(|i| format!("data:image/png;base64,{}", STANDARD.encode(i)))
                    .collect::<Vec<_>>()
            );
            Json(data).into_response()
        }
        Err(error) => failure(error),
    }
}
async fn presentation(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Some(id) = headers.get("x-retro-art-workbench")
        && let Err(error) = state.access.check_instance(id.to_str().unwrap_or(""))
    {
        return failure(error);
    }
    match state
        .workspace
        .lock()
        .map_err(|_| ArtError::new("storage_error", "Failed to acquire lock"))
        .and_then(|w| w.inspect_presentation(Default::default()))
    {
        Ok(output) => Json(output.data).into_response(),
        Err(error) => failure(error),
    }
}
async fn human_action(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(input): Json<dotmend::presentation::HumanAction>,
) -> Response {
    let id = headers
        .get("x-retro-art-workbench")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let action_id = headers
        .get("x-dotmend-action")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    let result = tokio::task::spawn_blocking(move || {
        state
            .access
            .human_action(&id, action_id.as_deref(), input, false, state.workspace)
    })
    .await;
    match result {
        Ok(Ok(output)) => Json(output.data).into_response(),
        Ok(Err(error)) => failure(error),
        Err(error) => failure(ArtError::new("storage_error", error.to_string())),
    }
}
async fn recover_action(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(input): Json<dotmend::presentation::HumanAction>,
) -> Response {
    let id = headers
        .get("x-retro-art-workbench")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let action_id = headers
        .get("x-dotmend-action")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    match tokio::task::spawn_blocking(move || {
        state
            .access
            .human_action(&id, action_id.as_deref(), input, true, state.workspace)
    })
    .await
    {
        Ok(Ok(output)) => Json(output.data).into_response(),
        Ok(Err(error)) => failure(error),
        Err(error) => failure(ArtError::new("storage_error", error.to_string())),
    }
}
async fn human_activity(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let id = headers
        .get("x-retro-art-workbench")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    match state.access.apply(id, || Ok(())) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(error) => failure(error),
    }
}
async fn art_data(State(state): State<WebState>, Path(id): Path<String>) -> Response {
    match state.workspace.lock().map_err(|_| ArtError::new("storage_error","Failed to acquire lock")).and_then(|w| w.load(&id)) {
        Ok(art) => Json(json!({"art_id":id,"target":art.target,"indices":art.indices,"context":art.context,"references":art.references,"parents":art.parents})).into_response(),
        Err(error) => failure(error),
    }
}
async fn art_image(State(state): State<WebState>, Path(id): Path<String>) -> Response {
    match state
        .workspace
        .lock()
        .map_err(|_| ArtError::new("storage_error", "Failed to acquire lock"))
        .and_then(|w| w.load(&id))
        .and_then(|art| encode_png(&art_raster(&art)?))
    {
        Ok(bytes) => (
            [
                ("content-type", "image/png"),
                ("cache-control", "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => failure(error),
    }
}
async fn source_image(State(state): State<WebState>, Path(hash): Path<String>) -> Response {
    match state
        .workspace
        .lock()
        .map_err(|_| ArtError::new("storage_error", "Failed to acquire lock"))
        .and_then(|w| w.source(&hash))
    {
        Ok(bytes) => (
            [
                ("content-type", "image/png"),
                ("cache-control", "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => failure(error),
    }
}

async fn download(
    State(state): State<WebState>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    match state
        .workspace
        .lock()
        .map_err(|_| ArtError::new("storage_error", "Failed to acquire lock"))
        .and_then(|w| w.resource(query.get("uri").map_or("", String::as_str)))
    {
        Ok((bytes, mime)) => ([("content-type", mime)], bytes).into_response(),
        Err(error) => failure(error),
    }
}
async fn control_call(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(request): Json<ToolCall>,
) -> Response {
    use dotmend::workbench_protocol::*;
    if headers.contains_key("origin")
        || headers
            .get("x-retro-art-control")
            .and_then(|v| v.to_str().ok())
            != Some("mcp")
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    fn decode<T: serde::de::DeserializeOwned>(v: Value) -> ArtResult<T> {
        serde_json::from_value(v).map_err(|e| invalid(e.to_string()))
    }
    let result: ArtResult<ToolOutput> = async {
        match request.tool.as_str() {
            "open_workbench" => {
                let input: OpenWorkbench = decode(request.arguments)?;
                let idle = input.idle_timeout_seconds.unwrap_or(DEFAULT_IDLE_SECONDS);
                if !(1..=DEFAULT_IDLE_SECONDS).contains(&idle) {
                    return Err(invalid("idle_timeout_seconds must be 1..1800"));
                }
                state.access.reopen(&input.control_id, input.work_state)
            }
            "inspect_workbench" => {
                let input: InspectWorkbench = decode(request.arguments)?;
                state.access.inspect(&input.control_id)
            }
            "close_workbench" => state.access.request_close(decode(request.arguments)?),
            "present_art" => {
                state
                    .access
                    .present(decode(request.arguments)?, state.workspace)
                    .await
            }
            _ => Err(invalid("Unknown workbench control operation")),
        }
    }
    .await;
    match result {
        Ok(output) => Json(output.data).into_response(),
        Err(error) => failure(error),
    }
}
fn allowed_headers(headers: &HeaderMap, port: u16) -> bool {
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    let valid_host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| hosts.iter().any(|host| host == h));
    valid_host
        && headers.get("origin").is_none_or(|h| {
            h.to_str()
                .ok()
                .is_some_and(|origin| hosts.iter().any(|host| origin == format!("http://{host}")))
        })
}
async fn local_only(
    State(state): State<WebState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if !allowed_headers(request.headers(), state.port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response.headers_mut().insert("content-security-policy","default-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'".parse().unwrap());
    response
}
pub(crate) fn web_router(
    workspace: SharedWorkspace,
    port: u16,
    access: Arc<crate::workbench::WorkbenchAccess>,
) -> Router {
    let state = WebState {
        workspace,
        port,
        access,
    };
    Router::new()
        .route("/internal/control", post(control_call))
        .route(
            "/api/workbench",
            get(|State(state): State<WebState>| async move { Json(state.access.instance.clone()) }),
        )
        .route("/api/workbench/activity", post(human_activity))
        .route(
            "/",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [("content-type", "application/javascript")],
                    include_str!("../web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [("content-type", "text/css")],
                    include_str!("../web/style.css"),
                )
            }),
        )
        .route("/api/presentation", get(presentation))
        .route("/api/presentation/action", post(human_action))
        .route("/api/presentation/recover", post(recover_action))
        .route("/api/call", post(tool_call))
        .route("/api/art/{id}", get(art_data))
        .route("/api/art/{id}/image", get(art_image))
        .route("/api/source/{hash}", get(source_image))
        .route("/api/download", get(download))
        .route("/api/tools", get(|| async { Json(json!(mcp_tools())) }))
        .layer(DefaultBodyLimit::max(MAX_SOURCE_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), local_only))
        .with_state(state)
}

impl ArtServer {
    async fn managed_call(&self, name: &str, arguments: Value) -> ArtResult<ToolOutput> {
        let _permit = self.invocations.enter()?;
        use dotmend::workbench_protocol::*;
        fn decode<T: serde::de::DeserializeOwned>(input: Value) -> ArtResult<T> {
            serde_json::from_value(input).map_err(|e| invalid(e.to_string()))
        }
        match name {
            "open_workbench" => {
                self.workbench
                    .lock()
                    .await
                    .open(decode(arguments)?, self.workspace.clone())
                    .await
            }
            "inspect_workbench" => {
                let input: InspectWorkbench = decode(arguments)?;
                self.workbench.lock().await.inspect(&input.control_id).await
            }
            "close_workbench" => self.workbench.lock().await.close(decode(arguments)?).await,
            "present_art" => {
                let input: ManagedPresentArt = decode(arguments)?;
                self.workbench
                    .lock()
                    .await
                    .present(input, self.workspace.clone())
                    .await
            }
            _ => execute(self.workspace.clone(), name.into(), arguments).await,
        }
    }
}
