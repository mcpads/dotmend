use crate::{art::*, focus::magnify, pixels::*, workflow::*};
use serde_json::{Value, json};

pub struct FrameOutput {
    pub data: Value,
    pub images: Vec<Vec<u8>>,
}
pub fn check_frames(frames: &[Frame], arts: &[Art]) -> ArtResult<()> {
    if frames.is_empty() || frames.len() > MAX_SET_SIZE || frames.len() != arts.len() {
        return Err(invalid("Provide 1..16 frames"));
    }
    let anchored = frames[0].anchor.is_some();
    for (frame, art) in frames.iter().zip(arts) {
        art.check()?;
        if frame.art_id != art.id()? || frame.label.trim().is_empty() || frame.label.len() > 512 {
            return Err(invalid("Check frame IDs and labels"));
        }
        if frame.anchor.is_some() != anchored {
            return Err(invalid(
                "Specify anchors for every frame or omit all anchors",
            ));
        }
        if let Some(p) = frame.anchor
            && (p.x > art.target.width || p.y > art.target.height)
        {
            return Err(invalid("Frame anchor is outside the canvas"));
        }
        if frame.duration_ms == Some(0) {
            return Err(invalid("Frame duration must be a positive integer"));
        }
    }
    Ok(())
}
pub fn render_frames(arts: &[Art], input: &RenderArtSet) -> ArtResult<FrameOutput> {
    check_frames(&input.frames, arts)?;
    if !(1..=64).contains(&input.scale) {
        return Err(ArtError::new("limit_exceeded", "Scale must be 1..64"));
    }
    let left = input
        .frames
        .iter()
        .map(|f| f.anchor.map_or(0, |p| p.x))
        .max()
        .unwrap_or(0);
    let top = input
        .frames
        .iter()
        .map(|f| f.anchor.map_or(0, |p| p.y))
        .max()
        .unwrap_or(0);
    let right = input
        .frames
        .iter()
        .zip(arts)
        .map(|(f, a)| a.target.width - f.anchor.map_or(0, |p| p.x))
        .max()
        .unwrap_or(0);
    let bottom = input
        .frames
        .iter()
        .zip(arts)
        .map(|(f, a)| a.target.height - f.anchor.map_or(0, |p| p.y))
        .max()
        .unwrap_or(0);
    let width = left + right;
    let height = top + bottom;
    let pixels = width as u64 * height as u64 * (input.scale as u64).pow(2) * arts.len() as u64;
    if pixels > MAX_PREVIEW_PIXELS as u64 {
        return Err(ArtError::new(
            "limit_exceeded",
            "Combined frame display area exceeds the limit",
        )
        .detail(json!({"display_pixels":pixels,"max_preview_pixels":MAX_PREVIEW_PIXELS})));
    }
    let sheet = matches!(input.view, SetView::Sheet);
    let output_width = if sheet {
        width * arts.len() as u32
    } else {
        width
    };
    let mut images = vec![];
    let mut metadata = vec![];
    let mut canvas = Raster {
        width: output_width,
        height,
        rgba: vec![0; output_width as usize * height as usize * 4],
    };
    for (i, (frame, art)) in input.frames.iter().zip(arts).enumerate() {
        let dx = left - frame.anchor.map_or(0, |p| p.x) + if sheet { i as u32 * width } else { 0 };
        let dy = top - frame.anchor.map_or(0, |p| p.y);
        let raster = art_raster(art)?;
        for y in 0..raster.height {
            let start = ((dy + y) * output_width + dx) as usize * 4;
            let source = y as usize * raster.width as usize * 4;
            canvas.rgba[start..start + raster.width as usize * 4]
                .copy_from_slice(&raster.rgba[source..source + raster.width as usize * 4]);
        }
        metadata.push(json!({"art_id":frame.art_id,"label":frame.label,"image_index":if sheet {0}else{i},"region":art.target.bounds(),"display_region":{"x":dx*input.scale,"y":dy*input.scale,"width":art.target.width*input.scale,"height":art.target.height*input.scale},"scale":input.scale,"anchor":frame.anchor,"duration_ms":frame.duration_ms,"coordinate_system":"pixel_top_left_xy"}));
        if !sheet {
            images.push(encode_png(&magnify(&canvas, input.scale))?);
            canvas.rgba.fill(0);
        }
    }
    if sheet {
        images.push(encode_png(&magnify(&canvas, input.scale))?);
    }
    Ok(FrameOutput {
        data: json!({"frames":metadata,"view":input.view,"scale":input.scale,"display_pixels":pixels,"display_width":output_width*input.scale,"display_height":height*input.scale,"alignment":if input.frames[0].anchor.is_some(){"provided_anchors"}else{"top_left"},"playable":input.frames.iter().all(|f|f.duration_ms.is_some()),"implementation":implementation_id()}),
        images,
    })
}
