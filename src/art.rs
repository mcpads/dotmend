use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const MAX_PIXELS: usize = 262_144;
pub const MAX_DIMENSION: u32 = 1024;
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PREVIEW_PIXELS: usize = 1_048_576;
pub const MAX_OPERATIONS: usize = 256;
pub const MAX_EDIT_WRITES: usize = 1_048_576;
pub const MAX_SET_SIZE: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArtError {
    pub code: String,
    pub message: String,
    pub details: Value,
    pub retryable: bool,
}
impl ArtError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Value::Null,
            retryable: false,
        }
    }
    pub fn detail(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}
impl std::fmt::Display for ArtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ArtError {}
pub type ArtResult<T> = Result<T, ArtError>;
pub fn invalid(message: impl Into<String>) -> ArtError {
    ArtError::new("invalid_input", message)
}
pub fn encode_json(value: &impl Serialize) -> ArtResult<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn hash_json(value: &impl Serialize) -> ArtResult<String> {
    Ok(digest(&encode_json(value)?))
}

pub fn implementation_id() -> String {
    let mut hash = Sha256::new();
    for source in [
        include_bytes!("art.rs").as_slice(),
        include_bytes!("edit.rs").as_slice(),
        include_bytes!("validation.rs").as_slice(),
        include_bytes!("selection.rs").as_slice(),
        include_bytes!("workflow.rs").as_slice(),
        include_bytes!("frames.rs").as_slice(),
        include_bytes!("focus.rs").as_slice(),
        include_bytes!("pixels.rs").as_slice(),
        include_bytes!("requests.rs").as_slice(),
        include_bytes!("../Cargo.toml").as_slice(),
        include_bytes!("../Cargo.lock").as_slice(),
    ] {
        hash.update(source);
    }
    format!("sha256:{:x}", hash.finalize())
}

pub(crate) fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["id", "kind", "parameters", "source_ref"]))]
pub struct Requirement {
    pub id: String,
    pub kind: String,
    pub parameters: Value,
    #[serde(deserialize_with = "required_nullable")]
    pub source_ref: Option<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["resource_id", "width", "height", "palette", "transparent_index", "allowed_indices", "constraints_ref", "requirements"]))]
pub struct Target {
    pub resource_id: String,
    #[schemars(range(min = 1, max = 1024))]
    pub width: u32,
    #[schemars(range(min = 1, max = 1024))]
    pub height: u32,
    #[schemars(length(min = 1, max = 256))]
    /// Ordered #RRGGBB colors, including duplicates. Alpha is represented separately by transparent_index, not #RRGGBBAA.
    pub palette: Vec<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub transparent_index: Option<u16>,
    #[schemars(length(min = 1, max = 256))]
    pub allowed_indices: Vec<u16>,
    #[serde(deserialize_with = "required_nullable")]
    pub constraints_ref: Option<SourceRef>,
    #[schemars(length(max = 128))]
    pub requirements: Vec<Requirement>,
}

pub fn parse_rgb(color: &str) -> ArtResult<[u8; 3]> {
    if color.len() != 7
        || !color.starts_with('#')
        || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err(invalid("Colors must use #RRGGBB format"));
    }
    let value = u32::from_str_radix(&color[1..], 16).map_err(|_| invalid("Invalid RGB color"))?;
    Ok([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

impl Target {
    pub fn check(&self) -> ArtResult<()> {
        if self.resource_id.trim().is_empty() || self.resource_id.len() > 512 {
            return Err(invalid("resource_id must contain 1..512 bytes"));
        }
        if self.width == 0
            || self.height == 0
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
            || self.width as usize * self.height as usize > MAX_PIXELS
        {
            return Err(ArtError::new(
                "limit_exceeded",
                "Canvas dimensions must be 1..1024 with at most 262144 pixels",
            ));
        }
        if self.palette.is_empty() || self.palette.len() > 256 {
            return Err(invalid("The palette must contain 1..256 entries"));
        }
        for color in &self.palette {
            parse_rgb(color)?;
        }
        let allowed: BTreeSet<_> = self.allowed_indices.iter().copied().collect();
        if allowed.is_empty()
            || allowed.len() != self.allowed_indices.len()
            || allowed.iter().any(|i| *i as usize >= self.palette.len())
        {
            return Err(invalid(
                "allowed_indices must be nonempty, unique and within the palette",
            ));
        }
        if self
            .transparent_index
            .is_some_and(|i| i as usize >= self.palette.len())
        {
            return Err(invalid("The transparent index is outside the palette"));
        }
        if self.requirements.len() > 128 || encode_json(self)?.len() > 32 * 1024 {
            return Err(ArtError::new(
                "limit_exceeded",
                "Target constraints exceed the size limit",
            ));
        }
        let mut ids = BTreeSet::new();
        for req in &self.requirements {
            if req.id.is_empty() || req.kind.is_empty() || !ids.insert(&req.id) {
                return Err(invalid(
                    "Additional constraint IDs must be nonempty and unique",
                ));
            }
            if let Some(reference) = &req.source_ref {
                check_source_ref(reference)?;
            }
        }
        crate::validation::check_parameters(self)?;
        if let Some(reference) = &self.constraints_ref {
            check_source_ref(reference)?;
        }
        Ok(())
    }
    pub fn check_index(&self, index: u16) -> ArtResult<()> {
        if !self.allowed_indices.contains(&index) {
            return Err(
                ArtError::new("invalid_index", "Palette index is not allowed")
                    .detail(json!({"index":index,"allowed_indices":self.allowed_indices})),
            );
        }
        Ok(())
    }
    pub fn bounds(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        }
    }
}
pub(crate) fn check_source_ref(reference: &SourceRef) -> ArtResult<()> {
    if reference.id.is_empty()
        || reference.sha256.len() != 64
        || !reference.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid("Source provenance requires an ID and SHA-256 hash"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub fn check_within(&self, outer: Rect) -> ArtResult<()> {
        if self.width == 0
            || self.height == 0
            || self.x < outer.x
            || self.y < outer.y
            || self.x as u64 + self.width as u64 > outer.x as u64 + outer.width as u64
            || self.y as u64 + self.height as u64 > outer.y as u64 + outer.height as u64
        {
            return Err(ArtError::new(
                "out_of_bounds",
                "The requested region is outside the allowed bounds",
            )
            .detail(json!({"region":self,"bounds":outer})));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtContext {
    pub group_id: Option<String>,
    pub variant: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Reference {
    pub source_hash: String,
    pub label: String,
    pub role: ReferenceRole,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceRole {
    Original,
    Reference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Art {
    pub target: Target,
    pub indices: Vec<Vec<u16>>,
    pub context: ArtContext,
    pub references: Vec<Reference>,
    pub parents: Vec<String>,
    pub provenance: Value,
}
impl Art {
    pub fn check(&self) -> ArtResult<()> {
        self.target.check()?;
        if self.indices.len() != self.target.height as usize
            || self
                .indices
                .iter()
                .any(|r| r.len() != self.target.width as usize)
        {
            return Err(invalid("Index array dimensions do not match the target"));
        }
        for (y, row) in self.indices.iter().enumerate() {
            for (x, index) in row.iter().enumerate() {
                self.target.check_index(*index).map_err(|mut e| {
                    e.details["location"] = json!({"x":x,"y":y});
                    e
                })?;
            }
        }
        if self.references.len() > 32
            || encode_json(&self.context)?.len() > 4096
            || encode_json(&self.provenance)?.len() > 256 * 1024
        {
            return Err(ArtError::new(
                "limit_exceeded",
                "Art references or provenance exceed the size limit",
            ));
        }
        Ok(())
    }
    pub fn id(&self) -> ArtResult<String> {
        Ok(format!("art_{}", hash_json(self)?))
    }
    pub fn summary(&self) -> ArtResult<Value> {
        Ok(
            json!({"art_id":self.id()?,"resource_id":self.target.resource_id,"width":self.target.width,"height":self.target.height,"parents":self.parents,"context":self.context,"origin":self.provenance["operation"]}),
        )
    }
}

pub fn validate(art: &Art) -> ArtResult<Value> {
    serde_json::to_value(crate::validation::inspect_constraints(art)?)
        .map_err(|e| invalid(e.to_string()))
}

pub fn limits() -> Value {
    json!({"max_dimension":MAX_DIMENSION,"max_pixels":MAX_PIXELS,"max_palette":256,"max_operations":MAX_OPERATIONS,"max_edit_writes":MAX_EDIT_WRITES,"max_set_size":MAX_SET_SIZE,"max_preview_pixels":MAX_PREVIEW_PIXELS,"max_indices_reply":4096,"max_concern_pixels":crate::presentation::MAX_CONCERN_PIXELS,"max_source_bytes":MAX_SOURCE_BYTES,"max_source_pixels":16_777_216})
}
