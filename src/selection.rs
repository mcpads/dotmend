use crate::{art::*, validation::include_point};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeSet, VecDeque};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    Rect,
    Pixels {
        points: Vec<Point>,
    },
    ConnectedIndices {
        seed: Point,
        indices: Vec<u16>,
        connectivity: u8,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSelection {
    pub art_id: String,
    pub region: Rect,
    pub selector: Selector,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub y: u32,
    pub x: u32,
    pub width: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub base_art_id: String,
    pub target_hash: String,
    pub region: Rect,
    pub selector: Selector,
    pub bounds: Rect,
    pub pixel_count: usize,
    pub spans: Vec<Span>,
}
impl Selection {
    pub fn id(&self) -> ArtResult<String> {
        Ok(format!("selection_{}", hash_json(self)?))
    }
    pub fn summary(&self) -> ArtResult<serde_json::Value> {
        let id = self.id()?;
        Ok(
            json!({"selection_id":id,"base_art_id":self.base_art_id,"target_hash":self.target_hash,"region":self.region,"bounds":self.bounds,"pixel_count":self.pixel_count,"mask_uri":format!("dotmend://selections/{id}")}),
        )
    }
    pub fn points(&self) -> impl Iterator<Item = Point> + '_ {
        self.spans
            .iter()
            .flat_map(|s| (s.x..s.x + s.width).map(move |x| Point { x, y: s.y }))
    }
}
pub fn create_selection(art: &Art, mut input: CreateSelection) -> ArtResult<Selection> {
    art.check()?;
    if art.id()? != input.art_id {
        return Err(invalid("Selection base candidate does not match"));
    }
    input.region.check_within(art.target.bounds())?;
    let r = input.region;
    let mut selected = BTreeSet::new();
    let check = |p: Point| {
        Rect {
            x: p.x,
            y: p.y,
            width: 1,
            height: 1,
        }
        .check_within(r)
    };
    match &mut input.selector {
        Selector::Rect => {
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    selected.insert((y, x));
                }
            }
        }
        Selector::Pixels { points } => {
            if points.len() > 4096 {
                return Err(ArtError::new(
                    "limit_exceeded",
                    "Explicit selections may contain at most 4096 pixels",
                ));
            }
            points.sort_by_key(|p| (p.y, p.x));
            points.dedup();
            for &p in points.iter() {
                check(p)?;
                selected.insert((p.y, p.x));
            }
        }
        Selector::ConnectedIndices {
            seed,
            indices,
            connectivity,
        } => {
            check(*seed)?;
            if ![4, 8].contains(connectivity) {
                return Err(invalid("connectivity must be 4 or 8")
                    .detail(json!({"field":"selector.connectivity"})));
            }
            indices.sort_unstable();
            indices.dedup();
            if indices.is_empty() {
                return Err(invalid("Selection indices must not be empty"));
            }
            for i in indices.iter() {
                art.target.check_index(*i)?;
            }
            if !indices.contains(&art.indices[seed.y as usize][seed.x as usize]) {
                return Err(invalid("The seed pixel does not use a selected index"));
            }
            let mut seen = vec![false; art.target.width as usize * art.target.height as usize];
            let mut queue = VecDeque::from([*seed]);
            seen[seed.y as usize * art.target.width as usize + seed.x as usize] = true;
            while let Some(p) = queue.pop_front() {
                if !indices.contains(&art.indices[p.y as usize][p.x as usize]) {
                    continue;
                }
                selected.insert((p.y, p.x));
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        if (dx == 0 && dy == 0) || (*connectivity == 4 && dx != 0 && dy != 0) {
                            continue;
                        }
                        let x = p.x as i64 + dx;
                        let y = p.y as i64 + dy;
                        if x < r.x as i64
                            || y < r.y as i64
                            || x >= (r.x + r.width) as i64
                            || y >= (r.y + r.height) as i64
                        {
                            continue;
                        }
                        let offset = y as usize * art.target.width as usize + x as usize;
                        if !seen[offset] {
                            seen[offset] = true;
                            queue.push_back(Point {
                                x: x as u32,
                                y: y as u32,
                            });
                        }
                    }
                }
            }
        }
    }
    if selected.is_empty() {
        return Err(invalid("Selection must not be empty"));
    }
    let mut bounds = None;
    let mut spans: Vec<Span> = vec![];
    for &(y, x) in &selected {
        include_point(&mut bounds, x, y);
        if let Some(s) = spans.last_mut().filter(|s| s.y == y && s.x + s.width == x) {
            s.width += 1;
        } else {
            spans.push(Span { x, y, width: 1 });
        }
    }
    Ok(Selection {
        base_art_id: input.art_id,
        target_hash: hash_json(&art.target)?,
        region: r,
        selector: input.selector,
        bounds: bounds.expect("nonempty selection"),
        pixel_count: selected.len(),
        spans,
    })
}
pub struct PixelGuard {
    pub request_id: String,
    width: u32,
    allowed: Vec<bool>,
    protected: Vec<Option<String>>,
    write_selection_id: Option<String>,
}
impl PixelGuard {
    pub fn new(
        request_id: String,
        base: &Art,
        region: Rect,
        write: Option<&Selection>,
        protected: &[Selection],
    ) -> ArtResult<Self> {
        region.check_within(base.target.bounds())?;
        let size = base.target.width as usize * base.target.height as usize;
        let mut guard = Self {
            request_id,
            width: base.target.width,
            allowed: vec![false; size],
            protected: vec![None; size],
            write_selection_id: write.map(Selection::id).transpose()?,
        };
        for selection in write.into_iter().chain(protected) {
            if selection.base_art_id != base.id()?
                || selection.target_hash != hash_json(&base.target)?
            {
                return Err(ArtError::new(
                    "selection_mismatch",
                    "Selection base or target does not match the request",
                )
                .detail(json!({"selection_id":selection.id()?,"base_art_id":base.id()?})));
            }
        }
        if let Some(s) = write {
            s.bounds.check_within(region)?;
            for p in s.points() {
                guard.allowed[(p.y * guard.width + p.x) as usize] = true;
            }
        } else {
            for y in region.y..region.y + region.height {
                for x in region.x..region.x + region.width {
                    guard.allowed[(y * guard.width + x) as usize] = true;
                }
            }
        }
        for s in protected {
            let id = s.id()?;
            for p in s.points() {
                let offset = (p.y * guard.width + p.x) as usize;
                guard.allowed[offset] = false;
                guard.protected[offset] = Some(id.clone());
            }
        }
        Ok(guard)
    }
    pub fn check(&self, x: u32, y: u32) -> ArtResult<()> {
        let offset = y as usize * self.width as usize + x as usize;
        if x >= self.width || offset >= self.allowed.len() {
            return Err(ArtError::new(
                "out_of_bounds",
                "Write is outside the canvas",
            ));
        }
        if self.allowed[offset] {
            return Ok(());
        }
        let protected = &self.protected[offset];
        Err(ArtError::new(if protected.is_some(){"protected_pixel"}else{"out_of_bounds"},"Pixel write is not permitted by the request").detail(json!({"x":x,"y":y,"request_id":self.request_id,"selection_id":protected.as_ref().or(self.write_selection_id.as_ref())})))
    }
    pub fn check_preserved(&self, base: &Art, result: &Art) -> ArtResult<()> {
        if base.target != result.target {
            return Err(ArtError::new(
                "target_mismatch",
                "Target constraints do not match the request",
            ));
        }
        for y in 0..base.target.height {
            for x in 0..base.target.width {
                if base.indices[y as usize][x as usize] != result.indices[y as usize][x as usize] {
                    self.check(x, y)?;
                }
            }
        }
        Ok(())
    }
}
