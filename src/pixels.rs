use crate::art::*;
use crate::edit::changed_region;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Cursor};

#[derive(Debug, Clone)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub fn decode_png(bytes: &[u8]) -> ArtResult<Raster> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(ArtError::new("limit_exceeded", "PNG input exceeds 16 MiB"));
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: 80 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(image_error)?;
    let info = reader.info();
    if info.bit_depth != png::BitDepth::Eight
        || !matches!(info.color_type, png::ColorType::Rgb | png::ColorType::Rgba)
        || info.icc_profile.is_some()
        || info.animation_control.is_some()
        || (info.srgb.is_none()
            && (info.source_gamma.is_some() || info.source_chromaticities.is_some()))
        || info.trns.is_some()
    {
        return Err(ArtError::new(
            "unsupported_image",
            "Expected a single-frame 8-bit RGB/RGBA PNG with sRGB or no color profile",
        ));
    }
    if info.width == 0 || info.height == 0 || info.width > 4096 || info.height > 4096 {
        return Err(ArtError::new(
            "limit_exceeded",
            "Input image dimensions must not exceed 4096 pixels",
        ));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| ArtError::new("limit_exceeded", "Image buffer exceeds the size limit"))?;
    let mut buffer = vec![0; size];
    let output = reader.next_frame(&mut buffer).map_err(image_error)?;
    reader.finish().map_err(image_error)?;
    let rgba = if output.color_type == png::ColorType::Rgba {
        buffer[..output.buffer_size()].to_vec()
    } else {
        buffer[..output.buffer_size()]
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    };
    Ok(Raster {
        width: output.width,
        height: output.height,
        rgba,
    })
}
fn image_error(error: impl std::fmt::Display) -> ArtError {
    ArtError::new("unsupported_image", error.to_string())
}
pub fn encode_png(raster: &Raster) -> ArtResult<Vec<u8>> {
    if raster.width == 0
        || raster.height == 0
        || raster.rgba.len() as u64 != raster.width as u64 * raster.height as u64 * 4
    {
        return Err(invalid("Invalid RGBA buffer size"));
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, raster.width, raster.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(image_error)?;
        writer.write_image_data(&raster.rgba).map_err(image_error)?;
        writer.finish().map_err(image_error)?;
    }
    Ok(bytes)
}
pub fn palette_rgba(target: &Target) -> ArtResult<Vec<[u8; 4]>> {
    target
        .palette
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let rgb = parse_rgb(color)?;
            Ok([
                rgb[0],
                rgb[1],
                rgb[2],
                if Some(index as u16) == target.transparent_index {
                    0
                } else {
                    255
                },
            ])
        })
        .collect()
}
pub fn art_raster(art: &Art) -> ArtResult<Raster> {
    art.check()?;
    let palette = palette_rgba(&art.target)?;
    let rgba = art
        .indices
        .iter()
        .flatten()
        .flat_map(|index| palette[*index as usize])
        .collect();
    Ok(Raster {
        width: art.target.width,
        height: art.target.height,
        rgba,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Background {
    Checkerboard,
    Solid { rgb: String },
}
pub fn render(
    art: &Art,
    region: Option<Rect>,
    scale: u32,
    background: &Background,
    grid: bool,
) -> ArtResult<Vec<u8>> {
    encode_png(&render_raster(art, region, scale, background, grid)?)
}

pub(crate) fn render_raster(
    art: &Art,
    region: Option<Rect>,
    scale: u32,
    background: &Background,
    grid: bool,
) -> ArtResult<Raster> {
    art.check()?;
    let region = region.unwrap_or(art.target.bounds());
    region.check_within(art.target.bounds())?;
    if scale == 0
        || scale > 64
        || region.width as u64 * region.height as u64 * scale as u64 * scale as u64
            > MAX_PREVIEW_PIXELS as u64
    {
        return Err(ArtError::new(
            "limit_exceeded",
            "Scale must be 1..64 and display area must not exceed 1048576 pixels",
        ));
    }
    let solid = match background {
        Background::Solid { rgb } => Some(parse_rgb(rgb)?),
        _ => None,
    };
    let palette = palette_rgba(&art.target)?;
    let mut raster = Raster {
        width: region.width * scale,
        height: region.height * scale,
        rgba: Vec::new(),
    };
    for y in 0..raster.height {
        for x in 0..raster.width {
            let mut pixel = palette[art.indices[(region.y + y / scale) as usize]
                [(region.x + x / scale) as usize] as usize];
            if pixel[3] == 0 {
                let rgb = solid.unwrap_or_else(|| {
                    if (x / 8 + y / 8) % 2 == 0 {
                        [222; 3]
                    } else {
                        [190; 3]
                    }
                });
                pixel = [rgb[0], rgb[1], rgb[2], 255];
            }
            if grid && scale >= 3 && (x % scale == 0 || y % scale == 0) {
                pixel = [96, 112, 128, 255];
            }
            raster.rgba.extend_from_slice(&pixel);
        }
    }
    Ok(raster)
}

pub fn compare(before: &Art, after: &Art, region: Option<Rect>) -> ArtResult<(Value, Vec<u8>)> {
    let (data, raster) = compare_raster(before, after, region)?;
    Ok((data, encode_png(&raster)?))
}

pub(crate) fn compare_raster(
    before: &Art,
    after: &Art,
    region: Option<Rect>,
) -> ArtResult<(Value, Raster)> {
    before.check()?;
    after.check()?;
    if before.target != after.target {
        return Err(ArtError::new(
            "target_mismatch",
            "Comparison requires candidates with the same replacement target and constraints",
        ));
    }
    let region = region.unwrap_or(before.target.bounds());
    region.check_within(before.target.bounds())?;
    let palette = palette_rgba(&before.target)?;
    let mut visual = 0;
    let mut mask = Raster {
        width: region.width,
        height: region.height,
        rgba: Vec::new(),
    };
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            let a = before.indices[y as usize][x as usize];
            let b = after.indices[y as usize][x as usize];
            if palette[a as usize] != palette[b as usize]
                && !(palette[a as usize][3] == 0 && palette[b as usize][3] == 0)
            {
                visual += 1;
            }
            mask.rgba.extend_from_slice(if a == b {
                &[0, 0, 0, 0]
            } else {
                &[255, 80, 140, 255]
            });
        }
    }
    let (count, changed) = changed_region(before, after, region);
    Ok((
        json!({"before_art_id":before.id()?,"after_art_id":after.id()?,"region":region,"changed_pixels":count,"visual_changed_pixels":visual,"changed_region":changed}),
        mask,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Resize {
    None,
    Nearest,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Alpha {
    Threshold { cutoff: u8 },
    Matte { rgb: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColorMapping {
    NearestRgb { opaque_indices: Vec<u16> },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dither {
    None,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub crop: Rect,
    pub resize: Resize,
    pub alpha: Alpha,
    pub color_mapping: ColorMapping,
    pub dither: Dither,
}

pub fn quantize(
    source: &Raster,
    target: &Target,
    transform: &Transform,
) -> ArtResult<(Vec<Vec<u16>>, Value)> {
    target.check()?;
    if source.rgba.len() as u64 != source.width as u64 * source.height as u64 * 4 {
        return Err(invalid("Source RGBA buffer size does not match"));
    }
    transform.crop.check_within(Rect {
        x: 0,
        y: 0,
        width: source.width,
        height: source.height,
    })?;
    if matches!(transform.resize, Resize::None)
        && (transform.crop.width != target.width || transform.crop.height != target.height)
    {
        return Err(invalid(
            "With resize=none, cropped dimensions must match the target",
        ));
    }
    let ColorMapping::NearestRgb { opaque_indices } = &transform.color_mapping;
    let sorted: BTreeSet<u16> = opaque_indices.iter().copied().collect();
    if sorted.is_empty() || sorted.len() != opaque_indices.len() {
        return Err(invalid(
            "Opaque candidate indices must be nonempty and unique",
        ));
    }
    for index in &sorted {
        target.check_index(*index)?;
        if Some(*index) == target.transparent_index {
            return Err(invalid(
                "Opaque color candidates include the transparent index",
            ));
        }
    }
    let palette = palette_rgba(target)?;
    let matte = match &transform.alpha {
        Alpha::Matte { rgb } => Some(parse_rgb(rgb)?),
        _ => None,
    };
    let mut rows = vec![vec![0u16; target.width as usize]; target.height as usize];
    let mut transparent = 0u64;
    let mut ties = 0u64;
    let mut distances = 0u64;
    let mut max_distance = 0u32;
    for y in 0..target.height {
        for x in 0..target.width {
            let sx = transform.crop.x
                + (((2 * x as u64 + 1) * transform.crop.width as u64) / (2 * target.width as u64))
                    as u32;
            let sy = transform.crop.y
                + (((2 * y as u64 + 1) * transform.crop.height as u64) / (2 * target.height as u64))
                    as u32;
            let offset = (sy as usize * source.width as usize + sx as usize) * 4;
            let pixel = &source.rgba[offset..offset + 4];
            if matches!(transform.alpha, Alpha::Threshold { cutoff } if pixel[3] < cutoff) {
                let index = target
                    .transparent_index
                    .ok_or_else(|| invalid("No index is available for transparent pixels"))?;
                target.check_index(index)?;
                rows[y as usize][x as usize] = index;
                transparent += 1;
                continue;
            }
            let mut rgb = [pixel[0], pixel[1], pixel[2]];
            if let Some(matte) = matte {
                for channel in 0..3 {
                    rgb[channel] = ((pixel[channel] as u32 * pixel[3] as u32
                        + matte[channel] as u32 * (255 - pixel[3] as u32)
                        + 127)
                        / 255) as u8;
                }
            }
            let mut best = u32::MAX;
            let mut index = 0;
            let mut tie = false;
            for candidate in &sorted {
                let distance: u32 = (0..3)
                    .map(|c| (rgb[c] as i32 - palette[*candidate as usize][c] as i32).pow(2) as u32)
                    .sum();
                if distance < best {
                    best = distance;
                    index = *candidate;
                    tie = false;
                } else if distance == best {
                    tie = true;
                }
            }
            rows[y as usize][x as usize] = index;
            distances += best as u64;
            max_distance = max_distance.max(best);
            ties += u64::from(tie);
        }
    }
    let opaque = target.width as u64 * target.height as u64 - transparent;
    Ok((
        rows,
        json!({"source_width":source.width,"source_height":source.height,"width":target.width,"height":target.height,"transform":transform,"transparent_pixels":transparent,"tie_pixels":ties,"mean_squared_rgb_distance":if opaque > 0 { distances as f64 / opaque as f64 } else { 0.0 },"max_squared_rgb_distance":max_distance,"aspect_ratio_changed":transform.crop.width as u64 * target.height as u64 != transform.crop.height as u64 * target.width as u64}),
    ))
}
