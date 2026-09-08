use crate::art::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pixel {
    pub x: u32,
    pub y: u32,
    pub index: u16,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PasteMode {
    Replace,
    Over,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditOperation {
    SetPixels {
        pixels: Vec<Pixel>,
    },
    PaintRows {
        x: u32,
        y: u32,
        rows: Vec<Vec<Option<u16>>>,
    },
    FillRect {
        rect: Rect,
        index: u16,
    },
    ReplaceIndex {
        rect: Rect,
        from_index: u16,
        to_index: u16,
    },
    Paste {
        source_art_id: String,
        source_rect: Rect,
        x: u32,
        y: u32,
        flip_x: bool,
        flip_y: bool,
        mode: PasteMode,
    },
}

pub fn apply_edits(
    base: &Art,
    write_region: Rect,
    operations: &[EditOperation],
    source: impl FnMut(&str) -> ArtResult<Art>,
) -> ArtResult<Art> {
    apply_guarded_edits(base, write_region, operations, source, None)
}

pub fn apply_guarded_edits(
    base: &Art,
    write_region: Rect,
    operations: &[EditOperation],
    mut source: impl FnMut(&str) -> ArtResult<Art>,
    guard: Option<&crate::selection::PixelGuard>,
) -> ArtResult<Art> {
    base.check()?;
    write_region.check_within(base.target.bounds())?;
    if operations.is_empty() || operations.len() > MAX_OPERATIONS {
        return Err(ArtError::new(
            "limit_exceeded",
            "Provide 1..256 edit operations",
        ));
    }
    let check_write = |x, y| guard.map_or(Ok(()), |g| g.check(x, y));
    let mut result = base.clone();
    let mut writes = 0usize;
    for (operation_index, operation) in operations.iter().enumerate() {
        let mut apply = || -> ArtResult<()> {
            let mut check_area = |rect: Rect| -> ArtResult<()> {
                rect.check_within(write_region)?;
                writes += rect.width as usize * rect.height as usize;
                if writes > MAX_EDIT_WRITES {
                    return Err(ArtError::new(
                        "limit_exceeded",
                        "Pixel writes in one call exceed the limit",
                    ));
                }
                Ok(())
            };
            match operation {
                EditOperation::SetPixels { pixels } => {
                    for pixel in pixels {
                        check_area(Rect {
                            x: pixel.x,
                            y: pixel.y,
                            width: 1,
                            height: 1,
                        })?;
                        base.target.check_index(pixel.index)?;
                        check_write(pixel.x, pixel.y)?;
                        result.indices[pixel.y as usize][pixel.x as usize] = pixel.index;
                    }
                }
                EditOperation::PaintRows { x, y, rows } => {
                    let width = rows.first().map_or(0, Vec::len);
                    if width == 0
                        || rows.iter().any(|row| row.len() != width)
                        || width > MAX_DIMENSION as usize
                        || rows.len() > MAX_DIMENSION as usize
                    {
                        return Err(invalid("paint_rows must be a nonempty rectangle"));
                    }
                    check_area(Rect {
                        x: *x,
                        y: *y,
                        width: width as u32,
                        height: rows.len() as u32,
                    })?;
                    for (dy, row) in rows.iter().enumerate() {
                        for (dx, index) in row.iter().enumerate() {
                            if let Some(index) = index {
                                base.target.check_index(*index)?;
                                check_write(*x + dx as u32, *y + dy as u32)?;
                                result.indices[*y as usize + dy][*x as usize + dx] = *index;
                            }
                        }
                    }
                }
                EditOperation::FillRect { rect, index } => {
                    check_area(*rect)?;
                    base.target.check_index(*index)?;
                    for y in rect.y..rect.y + rect.height {
                        for x in rect.x..rect.x + rect.width {
                            check_write(x, y)?;
                            result.indices[y as usize][x as usize] = *index;
                        }
                    }
                }
                EditOperation::ReplaceIndex {
                    rect,
                    from_index,
                    to_index,
                } => {
                    check_area(*rect)?;
                    base.target.check_index(*from_index)?;
                    base.target.check_index(*to_index)?;
                    for y in rect.y..rect.y + rect.height {
                        for x in rect.x..rect.x + rect.width {
                            if result.indices[y as usize][x as usize] == *from_index {
                                check_write(x, y)?;
                                result.indices[y as usize][x as usize] = *to_index;
                            }
                        }
                    }
                }
                EditOperation::Paste {
                    source_art_id,
                    source_rect,
                    x,
                    y,
                    flip_x,
                    flip_y,
                    mode,
                } => {
                    check_area(Rect {
                        x: *x,
                        y: *y,
                        width: source_rect.width,
                        height: source_rect.height,
                    })?;
                    let other = source(source_art_id)?;
                    other.check()?;
                    source_rect.check_within(other.target.bounds())?;
                    if other.target.palette != base.target.palette
                        || other.target.transparent_index != base.target.transparent_index
                    {
                        return Err(ArtError::new(
                            "palette_mismatch",
                            "Paste source palette or transparent index does not match",
                        ));
                    }
                    for dy in 0..source_rect.height {
                        for dx in 0..source_rect.width {
                            let sx = source_rect.x
                                + if *flip_x {
                                    source_rect.width - dx - 1
                                } else {
                                    dx
                                };
                            let sy = source_rect.y
                                + if *flip_y {
                                    source_rect.height - dy - 1
                                } else {
                                    dy
                                };
                            let index = other.indices[sy as usize][sx as usize];
                            if matches!(mode, PasteMode::Over)
                                && Some(index) == other.target.transparent_index
                            {
                                continue;
                            }
                            base.target.check_index(index)?;
                            check_write(*x + dx, *y + dy)?;
                            result.indices[(*y + dy) as usize][(*x + dx) as usize] = index;
                        }
                    }
                }
            }
            Ok(())
        };
        apply().map_err(|mut error| {
            error.details = json!({"operation_index":operation_index,"cause":error.details});
            error
        })?;
    }
    if result.indices != base.indices {
        result.parents = vec![base.id()?];
        result.provenance = json!({"operation":"edit","write_region":write_region,"operations":operations,"implementation":implementation_id()});
    }
    if result.indices != base.indices
        && let Some(guard) = guard
    {
        result.provenance["request_id"] = json!(guard.request_id);
    }
    Ok(result)
}

pub fn changed_region(before: &Art, after: &Art, region: Rect) -> (usize, Option<Rect>) {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut count = 0;
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            if before.indices[y as usize][x as usize] != after.indices[y as usize][x as usize] {
                count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    (
        count,
        (count > 0).then(|| Rect {
            x: min_x,
            y: min_y,
            width: max_x - min_x + 1,
            height: max_y - min_y + 1,
        }),
    )
}
pub fn edit_summary(before: &Art, after: &Art) -> ArtResult<Value> {
    let (count, region) = changed_region(before, after, before.target.bounds());
    Ok(
        json!({"art_id":after.id()?,"base_art_id":before.id()?,"changed_pixels":count,"changed_region":region}),
    )
}
