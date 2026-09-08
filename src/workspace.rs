#[path = "workspace_presentation.rs"]
mod presentation;
#[path = "workspace_workflow.rs"]
mod workflow;
use crate::{art::*, edit::*, focus::focus_with_reference, pixels::*, requests::*};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub struct ToolOutput {
    pub data: Value,
    pub images: Vec<Vec<u8>>,
    pub links: Vec<String>,
}
impl ToolOutput {
    pub fn new(mut data: Value) -> Self {
        data["ok"] = json!(true);
        Self {
            data,
            images: vec![],
            links: vec![],
        }
    }
    fn image(mut self, png: Vec<u8>) -> Self {
        self.images.push(png);
        self
    }
}
pub struct Workspace {
    root: PathBuf,
    database: Connection,
}
fn insert_arts(database: &Connection, arts: &[Art]) -> ArtResult<()> {
    for art in arts {
        art.check()?;
        database
            .execute(
                "INSERT OR IGNORE INTO arts(id,resource_id,group_id,payload) VALUES (?1,?2,?3,?4)",
                params![
                    art.id()?,
                    art.target.resource_id,
                    art.context.group_id,
                    encode_json(art)?
                ],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}
fn storage_error(error: impl std::fmt::Display) -> ArtError {
    ArtError::new("storage_error", error.to_string())
}
fn decode<T: DeserializeOwned>(value: Value) -> ArtResult<T> {
    serde_json::from_value(value).map_err(|e| invalid(e.to_string()))
}

#[derive(Serialize, Deserialize)]
struct ArtCursor {
    upper: i64,
    after: i64,
    resource_id: Option<String>,
    group_id: Option<String>,
}

impl Workspace {
    pub fn open(root: impl AsRef<Path>) -> ArtResult<Self> {
        fs::create_dir_all(root.as_ref()).map_err(storage_error)?;
        let root = root.as_ref().canonicalize().map_err(storage_error)?;
        // Keep stored candidates and workspace locks shared with earlier installations.
        let directory = root.join(".retro-art");
        fs::create_dir_all(&directory).map_err(storage_error)?;
        let database = Connection::open(directory.join("art.sqlite")).map_err(storage_error)?;
        database
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage_error)?;
        let initialize = || {
            database.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS arts (seq INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, resource_id TEXT NOT NULL, group_id TEXT, payload BLOB NOT NULL);
            CREATE INDEX IF NOT EXISTS arts_resource ON arts(resource_id, seq);
            CREATE INDEX IF NOT EXISTS arts_group ON arts(group_id, seq);
            CREATE TABLE IF NOT EXISTS sources (hash TEXT PRIMARY KEY, payload BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS edit_requests (seq INTEGER PRIMARY KEY, id TEXT UNIQUE NOT NULL, payload BLOB NOT NULL, result_id TEXT, notes TEXT);")
        };
        // WAL setup can return SQLITE_BUSY without invoking SQLite's busy handler.
        // Only this idempotent bootstrap is retried; user actions are never replayed.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match initialize() {
                Ok(()) => break,
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if matches!(
                        error.code,
                        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                    ) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(error) => return Err(storage_error(error)),
            }
        }
        let transaction = database.unchecked_transaction().map_err(storage_error)?;
        transaction.execute_batch("CREATE TABLE IF NOT EXISTS selections (id TEXT PRIMARY KEY, payload BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS edit_reviews (seq INTEGER PRIMARY KEY, id TEXT UNIQUE NOT NULL, request_id TEXT NOT NULL, payload BLOB NOT NULL);
            CREATE INDEX IF NOT EXISTS reviews_request ON edit_reviews(request_id,seq);
            CREATE TABLE IF NOT EXISTS presentations (id TEXT PRIMARY KEY,payload BLOB NOT NULL,head_id TEXT NOT NULL,saved_state_id TEXT);
            CREATE TABLE IF NOT EXISTS presentation_states (id TEXT PRIMARY KEY,payload BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS presentation_saves (id TEXT PRIMARY KEY,payload BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS active_presentation (singleton INTEGER PRIMARY KEY CHECK(singleton=1),id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS presentation_actions (id TEXT PRIMARY KEY,state_id TEXT NOT NULL);").map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(Self { root, database })
    }
    pub fn load(&self, art_id: &str) -> ArtResult<Art> {
        let bytes: Option<Vec<u8>> = self
            .database
            .query_row("SELECT payload FROM arts WHERE id=?1", [art_id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage_error)?;
        let bytes = bytes.ok_or_else(|| {
            ArtError::new("art_not_found", "Art ID was not found").detail(json!({"art_id":art_id}))
        })?;
        let art: Art = serde_json::from_slice(&bytes).map_err(storage_error)?;
        if art.id()? != art_id {
            return Err(ArtError::new(
                "integrity_error",
                "Stored art hash does not match",
            ));
        }
        art.check()?;
        Ok(art)
    }
    fn save(&mut self, arts: &[Art], sources: &[(String, Vec<u8>)]) -> ArtResult<()> {
        for (hash, bytes) in sources {
            if digest(bytes) != *hash {
                return Err(invalid("Input image hash does not match"));
            }
        }
        let transaction = self.database.transaction().map_err(storage_error)?;
        for (hash, bytes) in sources {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO sources(hash,payload) VALUES (?1,?2)",
                    params![hash, bytes],
                )
                .map_err(storage_error)?;
        }
        insert_arts(&transaction, arts)?;
        transaction.commit().map_err(storage_error)
    }
    fn input_path(&self, path: &str) -> ArtResult<PathBuf> {
        let path = Path::new(path);
        if path.is_absolute()
            || path
                .components()
                .any(|p| !matches!(p, Component::Normal(_) | Component::CurDir))
        {
            return Err(invalid("Provide a relative path within the workspace"));
        }
        let resolved = self
            .root
            .join(path)
            .canonicalize()
            .map_err(|e| ArtError::new("source_unavailable", e.to_string()))?;
        if !resolved.starts_with(&self.root) {
            return Err(invalid("Input path points outside the workspace"));
        }
        Ok(resolved)
    }
    fn read_file(&self, path: &str) -> ArtResult<Vec<u8>> {
        let path = self.input_path(path)?;
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(storage_error)?
            .take((MAX_SOURCE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(storage_error)?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(ArtError::new("limit_exceeded", "Input file exceeds 16 MiB"));
        }
        Ok(bytes)
    }
    pub fn import_png(&mut self, bytes: Vec<u8>) -> ArtResult<Value> {
        let raster = decode_png(&bytes)?;
        let hash = digest(&bytes);
        self.save(&[], &[(hash.clone(), bytes)])?;
        Ok(
            json!({"source_path":format!("source:{hash}"),"source_hash":hash,"width":raster.width,"height":raster.height}),
        )
    }
    pub fn source(&self, hash: &str) -> ArtResult<Vec<u8>> {
        let bytes: Option<Vec<u8>> = self
            .database
            .query_row("SELECT payload FROM sources WHERE hash=?1", [hash], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage_error)?;
        let bytes = bytes
            .ok_or_else(|| ArtError::new("source_unavailable", "Stored image was not found"))?;
        if digest(&bytes) != hash {
            return Err(ArtError::new(
                "integrity_error",
                "Stored image hash does not match",
            ));
        }
        Ok(bytes)
    }
    fn image_source(&self, path: &str) -> ArtResult<Vec<u8>> {
        if let Some(hash) = path.strip_prefix("source:") {
            self.source(hash)
        } else {
            self.read_file(path)
        }
    }
    pub fn call(&mut self, name: &str, arguments: Value) -> ArtResult<ToolOutput> {
        let output = self.dispatch(name, arguments)?;
        crate::outputs::check_output(name, &output.data)?;
        Ok(output)
    }
    fn dispatch(&mut self, name: &str, arguments: Value) -> ArtResult<ToolOutput> {
        match name {
            "present_art" => self.present_art(decode(arguments)?),
            "inspect_presentation" => self.inspect_presentation(decode(arguments)?),
            "review_edit_result" => self.review_result(decode(arguments)?),
            "create_selection" => self.select_pixels(decode(arguments)?),
            "inspect_edit_request" => self.inspect_request(decode(arguments)?),
            "render_art_set" => {
                let request: crate::workflow::RenderArtSet = decode(arguments)?;
                let arts = request
                    .frames
                    .iter()
                    .map(|f| self.load(&f.art_id))
                    .collect::<ArtResult<Vec<_>>>()?;
                let result = crate::frames::render_frames(&arts, &request)?;
                let mut output = ToolOutput::new(result.data);
                output.images = result.images;
                Ok(output)
            }
            "create_art" => self.create(decode(arguments)?),
            "list_art" => self.list(decode(arguments)?),
            "inspect_art" => self.inspect(decode(arguments)?),
            "edit_art" => self.edit(decode(arguments)?),
            "edit_art_set" => self.edit_set(decode(arguments)?),
            "prepare_image" => self.prepare(decode(arguments)?),
            "attach_reference" => self.attach(decode(arguments)?),
            "render_art" => {
                let request: RenderArt = decode(arguments)?;
                let art = self.load(&request.art_id)?;
                let region = request.region.unwrap_or(art.target.bounds());
                let scale = request.scale.unwrap_or(1);
                let image = render(
                    &art,
                    Some(region),
                    scale,
                    &request.background.unwrap_or(Background::Checkerboard),
                    request.grid,
                )?;
                Ok(ToolOutput::new(json!({"art_id":request.art_id,"region":region,"scale":scale,"display_width":region.width * scale,"display_height":region.height * scale})).image(image))
            }
            "focus_art" => {
                let request: FocusArt = decode(arguments)?;
                let art = self.load(&request.art_id)?;
                let baseline = request
                    .compare_to_art_id
                    .as_ref()
                    .map(|id| self.load(id))
                    .transpose()?;
                let reference = request
                    .reference
                    .as_ref()
                    .map(|r| decode_png(&self.source(&r.source_hash)?))
                    .transpose()?;
                let focused =
                    focus_with_reference(&art, baseline.as_ref(), &request, reference.as_ref())?;
                let mut output = ToolOutput::new(focused.data);
                output.images = focused.images;
                output.links = art
                    .references
                    .iter()
                    .map(|reference| format!("dotmend://sources/{}", reference.source_hash))
                    .collect();
                Ok(output)
            }
            "compare_art" => {
                let request: CompareArt = decode(arguments)?;
                let (data, image) = compare(
                    &self.load(&request.before_art_id)?,
                    &self.load(&request.after_art_id)?,
                    request.region,
                )?;
                Ok(ToolOutput::new(data).image(image))
            }
            "validate_art" => {
                let request: ArtId = decode(arguments)?;
                Ok(ToolOutput::new(validate(&self.load(&request.art_id)?)?))
            }
            "export_art" => {
                let request: ArtId = decode(arguments)?;
                self.export(&request.art_id)
            }
            "request_edit" => self.request_edit(decode(arguments)?),
            "list_edit_requests" => self.list_requests(decode(arguments)?),
            "submit_edit_result" => self.submit_result(decode(arguments)?),
            _ => Err(ArtError::new("unknown_tool", "Unknown tool")),
        }
    }
    fn create(&mut self, request: CreateArt) -> ArtResult<ToolOutput> {
        let art = match (request.target, request.initial, request.bundle_path) {
            (Some(target), Some(initial), None) => {
                target.check()?;
                let indices = match &initial {
                    Initial::Fill { index } => {
                        target.check_index(*index)?;
                        vec![vec![*index; target.width as usize]; target.height as usize]
                    }
                    Initial::Indices { rows } => rows.clone(),
                };
                Art {
                    target,
                    indices,
                    context: request.context,
                    references: vec![],
                    parents: vec![],
                    provenance: json!({"operation":"create","initial_kind":match initial { Initial::Fill {..} => "fill", Initial::Indices {..} => "indices" },"implementation":implementation_id()}),
                }
            }
            (None, None, Some(bundle_path)) if request.context == ArtContext::default() => {
                self.read_bundle(&bundle_path)?
            }
            _ => {
                return Err(invalid(
                    "Specify either target with initial, or bundle_path",
                ));
            }
        };
        self.save(std::slice::from_ref(&art), &[])?;
        Ok(ToolOutput::new(
            json!({"art_id":art.id()?,"summary":art.summary()?,"provenance":art.provenance}),
        ))
    }
    fn list(&self, request: ListArt) -> ArtResult<ToolOutput> {
        let limit = request.limit.unwrap_or(30);
        if limit == 0 || limit > 100 {
            return Err(invalid("limit must be 1..100"));
        }
        let cursor = if let Some(cursor) = request.cursor {
            let bytes = STANDARD
                .decode(cursor)
                .map_err(|_| invalid("Invalid list cursor"))?;
            let cursor: ArtCursor =
                serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid list cursor"))?;
            if cursor.resource_id != request.resource_id || cursor.group_id != request.group_id {
                return Err(invalid(
                    "Search filters differ from the cursor's original filters",
                ));
            }
            cursor
        } else {
            ArtCursor {
                upper: self
                    .database
                    .query_row("SELECT COALESCE(MAX(seq),0) FROM arts", [], |r| r.get(0))
                    .map_err(storage_error)?,
                after: 0,
                resource_id: request.resource_id,
                group_id: request.group_id,
            }
        };
        let mut statement = self.database.prepare("SELECT seq,payload FROM arts WHERE seq>?1 AND seq<=?2 AND (?3 IS NULL OR resource_id=?3) AND (?4 IS NULL OR group_id=?4) ORDER BY seq LIMIT ?5").map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![
                    cursor.after,
                    cursor.upper,
                    cursor.resource_id,
                    cursor.group_id,
                    limit + 1
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .map_err(storage_error)?;
        let records: Vec<_> = rows.collect::<Result<_, _>>().map_err(storage_error)?;
        let mut arts = Vec::new();
        let mut after = cursor.after;
        for (seq, bytes) in records.iter().take(limit) {
            let art: Art = serde_json::from_slice(bytes).map_err(storage_error)?;
            arts.push(art.summary()?);
            after = *seq;
        }
        let next = if records.len() > limit {
            Some(STANDARD.encode(encode_json(&ArtCursor { after, ..cursor })?))
        } else {
            None
        };
        Ok(ToolOutput::new(
            json!({"arts":arts,"next_cursor":next,"limits":limits()}),
        ))
    }
    fn inspect(&self, request: InspectArt) -> ArtResult<ToolOutput> {
        let art = self.load(&request.art_id)?;
        let region = request.region.unwrap_or(art.target.bounds());
        region.check_within(art.target.bounds())?;
        if request.include_indices && region.width as usize * region.height as usize > 4096 {
            return Err(ArtError::new(
                "response_too_large",
                "Request regions of at most 4096 pixels",
            ));
        }
        let mut usage = vec![0usize; art.target.palette.len()];
        for index in art.indices.iter().flatten() {
            usage[*index as usize] += 1;
        }
        let rows = request.include_indices.then(|| {
            art.indices[region.y as usize..(region.y + region.height) as usize]
                .iter()
                .map(|r| r[region.x as usize..(region.x + region.width) as usize].to_vec())
                .collect::<Vec<_>>()
        });
        let links = art
            .references
            .iter()
            .map(|reference| format!("dotmend://sources/{}", reference.source_hash))
            .collect();
        let mut output = ToolOutput::new(
            json!({"art_id":request.art_id,"target":art.target,"context":art.context,"references":art.references,"parents":art.parents,"provenance":art.provenance,"usage":usage,"region":region,"indices":rows}),
        );
        output.links = links;
        Ok(output)
    }
    fn edit(&mut self, request: EditArt) -> ArtResult<ToolOutput> {
        let base = self.load(&request.art_id)?;
        let guard = request
            .request_id
            .as_ref()
            .map(|id| self.bound_guard(id, &base, request.write_region))
            .transpose()?;
        let edited = apply_guarded_edits(
            &base,
            request.write_region,
            &request.operations,
            |id| self.load(id),
            guard.as_ref(),
        )?;
        self.save(std::slice::from_ref(&edited), &[])?;
        Ok(ToolOutput::new(edit_summary(&base, &edited)?))
    }
    fn edit_set(&mut self, request: EditArtSet) -> ArtResult<ToolOutput> {
        if request.art_ids.is_empty()
            || request.art_ids.len() > MAX_SET_SIZE
            || request.art_ids.iter().collect::<BTreeSet<_>>().len() != request.art_ids.len()
        {
            return Err(invalid("Specify 1..16 distinct candidates"));
        }
        if let Some(bindings) = &request.request_bindings {
            let ids = bindings.iter().map(|b| &b.art_id).collect::<BTreeSet<_>>();
            if bindings.len() != request.art_ids.len()
                || ids.len() != bindings.len()
                || request.art_ids.iter().any(|id| !ids.contains(id))
            {
                return Err(invalid(
                    "Bind exactly one request to every candidate in the set",
                ));
            }
        }
        let first = self.load(&request.template_art_id)?;
        let mut candidates = Vec::new();
        let mut results = Vec::new();
        let mut images = Vec::new();
        for id in &request.art_ids {
            let candidate = (|| -> ArtResult<(Art, Value)> {
                let base = self.load(id)?;
                if first.target.width != base.target.width
                    || first.target.height != base.target.height
                    || first.target.palette != base.target.palette
                    || first.target.transparent_index != base.target.transparent_index
                    || first.target.allowed_indices != base.target.allowed_indices
                {
                    return Err(ArtError::new(
                        "target_mismatch",
                        "Dimensions, palette or allowed indices differ from the first target",
                    ));
                }
                let binding = request
                    .request_bindings
                    .as_ref()
                    .and_then(|bs| bs.iter().find(|b| b.art_id == *id));
                let guard = binding
                    .map(|b| self.bound_guard(&b.request_id, &base, request.write_region))
                    .transpose()?;
                let art = apply_guarded_edits(
                    &base,
                    request.write_region,
                    &request.operations,
                    |id| self.load(id),
                    guard.as_ref(),
                )?;
                let mut summary = edit_summary(&base, &art)?;
                summary["status"] = json!("compatible");
                summary["validation"] = validate(&art)?;
                Ok((art, summary))
            })();
            match candidate {
                Ok((art, mut summary)) => {
                    summary["image_index"] = json!(images.len());
                    images.push(render(&art, None, 1, &Background::Checkerboard, false)?);
                    candidates.push(art);
                    results.push(summary);
                }
                Err(error) => {
                    results.push(json!({"base_art_id":id,"status":"incompatible","error":error}))
                }
            }
        }
        let compatible = candidates.len() == request.art_ids.len();
        let plan_hash = compatible
            .then(|| {
                hash_json(
                    &json!({"template_art_id":request.template_art_id,"request_bindings":request.request_bindings,"candidates":candidates}),
                )
            })
            .transpose()?;
        if !request.preview {
            if !compatible {
                return Err(ArtError::new(
                    "validation_blocked",
                    "The entire set was left unchanged because a target is incompatible",
                )
                .detail(json!({"results":results})));
            }
            if request.expected_plan_hash != plan_hash {
                return Err(ArtError::new(
                    "plan_mismatch",
                    "Apply using the plan_hash returned by preview",
                ));
            }
            self.save(&candidates, &[])?;
        }
        Ok(ToolOutput {
            data: json!({"ok":true,"preview":request.preview,"compatible":compatible,"plan_hash":plan_hash,"results":results,"candidates_saved":!request.preview}),
            images,
            links: vec![],
        })
    }
    fn prepare(&mut self, request: PrepareImage) -> ArtResult<ToolOutput> {
        let base = self.load(&request.target_art_id)?;
        let bytes = self.image_source(&request.source_path)?;
        let source_hash = digest(&bytes);
        let source = decode_png(&bytes)?;
        let (indices, diagnostics) = quantize(&source, &base.target, &request.transform)?;
        let mut art = base.clone();
        art.indices = indices;
        art.parents = vec![base.id()?];
        art.references.push(Reference {
            source_hash: source_hash.clone(),
            label: "Conversion input".into(),
            role: ReferenceRole::Reference,
        });
        art.provenance = json!({"operation":"prepare_image","source_hash":source_hash,"transform":request.transform,"generation":request.provenance,"implementation":implementation_id()});
        self.save(std::slice::from_ref(&art), &[(source_hash, bytes)])?;
        Ok(ToolOutput::new(
            json!({"art_id":art.id()?,"diagnostics":diagnostics,"provenance":art.provenance}),
        ))
    }
    fn attach(&mut self, request: AttachReference) -> ArtResult<ToolOutput> {
        if request.label.is_empty() || request.label.len() > 512 {
            return Err(invalid("Reference label must contain 1..512 bytes"));
        }
        let mut art = self.load(&request.art_id)?;
        let bytes = self.image_source(&request.source_path)?;
        decode_png(&bytes)?;
        let hash = digest(&bytes);
        art.parents = vec![request.art_id];
        art.references.push(Reference {
            source_hash: hash.clone(),
            label: request.label,
            role: request.role,
        });
        art.provenance = json!({"operation":"attach_reference","source_hash":hash,"implementation":implementation_id()});
        self.save(std::slice::from_ref(&art), &[(hash, bytes)])?;
        Ok(ToolOutput::new(
            json!({"art_id":art.id()?,"references":art.references}),
        ))
    }
    fn export(&self, art_id: &str) -> ArtResult<ToolOutput> {
        let art = self.load(art_id)?;
        let validation = validate(&art)?;
        if validation["status"] != "pass" {
            return Err(ArtError::new(
                "validation_blocked",
                "All required checks must pass before export",
            )
            .detail(validation));
        }
        let mut files = BTreeMap::new();
        files.insert("art.json", encode_json(&json!({"target":art.target,"indices":art.indices,"context":art.context,"pixels_hash":hash_json(&art.indices)?,"target_hash":hash_json(&art.target)?}))?);
        files.insert("preview.png", encode_png(&art_raster(&art)?)?);
        files.insert("validation.json", encode_json(&validation)?);
        files.insert("provenance.json", encode_json(&json!({"art_id":art_id,"parents":art.parents,"references":art.references,"provenance":art.provenance}))?);
        let hashes: BTreeMap<_, _> = files
            .iter()
            .map(|(name, bytes)| (*name, digest(bytes)))
            .collect();
        files.insert("manifest.json", encode_json(&json!({"files":hashes}))?);
        let bundle_id = digest(&files["manifest.json"]);
        let parent = self.root.join(".retro-art/exports");
        fs::create_dir_all(&parent).map_err(storage_error)?;
        let destination = parent.join(&bundle_id);
        if destination.exists() {
            for (name, bytes) in &files {
                if fs::read(destination.join(name)).map_err(storage_error)? != *bytes {
                    return Err(ArtError::new(
                        "integrity_error",
                        "Existing export bundle contents have changed",
                    ));
                }
            }
        } else {
            let temp = tempfile::tempdir_in(&parent).map_err(storage_error)?;
            for (name, bytes) in &files {
                let mut file = fs::File::create(temp.path().join(name)).map_err(storage_error)?;
                file.write_all(bytes).map_err(storage_error)?;
                file.sync_all().map_err(storage_error)?;
            }
            fs::File::open(temp.path())
                .and_then(|directory| directory.sync_all())
                .map_err(storage_error)?;
            fs::rename(temp.path(), &destination).map_err(storage_error)?;
            fs::File::open(&parent)
                .and_then(|directory| directory.sync_all())
                .map_err(storage_error)?;
        }
        let links: Vec<_> = files
            .keys()
            .map(|name| format!("dotmend://exports/{bundle_id}/{name}"))
            .collect();
        Ok(ToolOutput {
            data: json!({"ok":true,"art_id":art_id,"bundle_id":bundle_id,"bundle_path":format!(".retro-art/exports/{bundle_id}"),"files":links,"validation":validation}),
            images: vec![],
            links,
        })
    }
    fn read_bundle(&self, path: &str) -> ArtResult<Art> {
        let manifest_bytes = self.read_file(&format!("{path}/manifest.json"))?;
        let manifest: Value =
            serde_json::from_slice(&manifest_bytes).map_err(|e| invalid(e.to_string()))?;
        let expected = [
            "art.json",
            "preview.png",
            "validation.json",
            "provenance.json",
        ];
        let hashes = manifest["files"]
            .as_object()
            .ok_or_else(|| invalid("Bundle file list is missing"))?;
        if hashes.len() != expected.len() || expected.iter().any(|name| !hashes.contains_key(*name))
        {
            return Err(invalid("Bundle does not contain the required files"));
        }
        let mut files = BTreeMap::new();
        for name in expected {
            let bytes = self.read_file(&format!("{path}/{name}"))?;
            if hashes[name].as_str() != Some(&digest(&bytes)) {
                return Err(ArtError::new(
                    "integrity_error",
                    format!("Bundle file hash does not match: {name}"),
                ));
            }
            files.insert(name, bytes);
        }
        let data: Value =
            serde_json::from_slice(&files["art.json"]).map_err(|e| invalid(e.to_string()))?;
        let art = Art {
            target: decode(data["target"].clone())?,
            indices: decode(data["indices"].clone())?,
            context: decode(data["context"].clone())?,
            references: vec![],
            parents: vec![],
            provenance: json!({"operation":"import_bundle","manifest_hash":digest(&manifest_bytes),"imported_provenance":serde_json::from_slice::<Value>(&files["provenance.json"]).map_err(storage_error)?,"prior_validation":serde_json::from_slice::<Value>(&files["validation.json"]).map_err(storage_error)?,"implementation":implementation_id()}),
        };
        art.check()?;
        if data["pixels_hash"] != hash_json(&art.indices)?
            || data["target_hash"] != hash_json(&art.target)?
        {
            return Err(ArtError::new(
                "integrity_error",
                "Bundle data hash does not match",
            ));
        }
        let preview = decode_png(&files["preview.png"])?;
        let raster = art_raster(&art)?;
        if preview.width != raster.width
            || preview.height != raster.height
            || preview.rgba != raster.rgba
        {
            return Err(ArtError::new(
                "integrity_error",
                "Preview pixels do not match the index data",
            ));
        }
        Ok(art)
    }
    pub fn resource(&self, uri: &str) -> ArtResult<(Vec<u8>, &'static str)> {
        // Previously issued resource links remain readable after the product rename.
        let path = uri
            .strip_prefix("dotmend://")
            .or_else(|| uri.strip_prefix("retro-art://"))
            .ok_or_else(|| ArtError::new("resource_not_found", "Resource URI is not available"))?;
        if path == "guides/editing" {
            return Ok((
                include_bytes!("../resources/agent-usage.md").to_vec(),
                "text/markdown",
            ));
        }
        if let Some(id) = path.strip_prefix("selections/") {
            return Ok((encode_json(&self.load_selection(id)?)?, "application/json"));
        }
        if let Some(hash) = path.strip_prefix("sources/") {
            return Ok((self.source(hash)?, "image/png"));
        }
        let path = path
            .strip_prefix("exports/")
            .ok_or_else(|| ArtError::new("resource_not_found", "Resource URI is not available"))?;
        let (id, name) = path
            .split_once('/')
            .ok_or_else(|| invalid("Invalid export URI"))?;
        if id.len() != 64
            || !id.bytes().all(|b| b.is_ascii_hexdigit())
            || ![
                "art.json",
                "preview.png",
                "validation.json",
                "provenance.json",
                "manifest.json",
            ]
            .contains(&name)
        {
            return Err(invalid("Invalid export URI"));
        }
        Ok((
            self.read_file(&format!(".retro-art/exports/{id}/{name}"))?,
            if name.ends_with(".png") {
                "image/png"
            } else {
                "application/json"
            },
        ))
    }
}
