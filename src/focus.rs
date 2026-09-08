use crate::{art::*, pixels::*, requests::FocusArt};
use serde_json::{Value, json};

pub struct FocusOutput {
    pub data: Value,
    pub images: Vec<Vec<u8>>,
}

pub fn focus(art: &Art, baseline: Option<&Art>, request: &FocusArt) -> ArtResult<FocusOutput> {
    focus_with_reference(art, baseline, request, None)
}

pub fn focus_with_reference(
    art: &Art,
    baseline: Option<&Art>,
    request: &FocusArt,
    reference: Option<&Raster>,
) -> ArtResult<FocusOutput> {
    art.check()?;
    if request.reference.is_some() != reference.is_some() {
        return Err(invalid(
            "Provide both the reference region and its source image",
        ));
    }
    let reference_pixels = if let (Some(input), Some(source)) = (&request.reference, reference) {
        if !art
            .references
            .iter()
            .any(|r| r.source_hash == input.source_hash)
        {
            return Err(invalid("Reference is not attached to this candidate"));
        }
        input.region.check_within(Rect {
            x: 0,
            y: 0,
            width: source.width,
            height: source.height,
        })?;
        if !(1..=64).contains(&input.scale) {
            return Err(ArtError::new(
                "limit_exceeded",
                "Reference scale must be 1..64",
            ));
        }
        input.region.width as u64 * input.region.height as u64 * (input.scale as u64).pow(2)
    } else {
        0
    };
    let art_id = art.id()?;
    if request.art_id != art_id || request.compare_to_art_id != baseline.map(Art::id).transpose()? {
        return Err(invalid("Focus request art_id does not match the input art"));
    }
    if let Some(before) = baseline {
        before.check()?;
        if before.target != art.target {
            return Err(ArtError::new(
                "target_mismatch",
                "Focus comparison requires candidates with the same replacement target and constraints",
            ));
        }
    }
    let region = request.region;
    region.check_within(art.target.bounds())?;
    let padding = request.context_padding;
    let x = region.x.saturating_sub(padding);
    let y = region.y.saturating_sub(padding);
    let right = (region.x + region.width)
        .saturating_add(padding)
        .min(art.target.width);
    let bottom = (region.y + region.height)
        .saturating_add(padding)
        .min(art.target.height);
    let context = Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    };
    let clipped = json!({
        "left":padding.saturating_sub(region.x - x),
        "top":padding.saturating_sub(region.y - y),
        "right":padding.saturating_sub(right - region.x - region.width),
        "bottom":padding.saturating_sub(bottom - region.y - region.height)
    });
    if !(1..=64).contains(&request.scale) {
        return Err(ArtError::new(
            "limit_exceeded",
            "Focus scale must be an integer from 1 to 64",
        )
        .detail(json!({"field":"scale","max_scale":64})));
    }
    if request.grid && request.scale < 3 {
        return Err(
            invalid("Use scale >= 3 for the grid, or request grid=false")
                .detail(json!({"field":"scale","min_grid_scale":3})),
        );
    }
    let detail_count = 1 + u64::from(request.grid) + if baseline.is_some() { 2 } else { 0 };
    let display_pixels = reference_pixels
        + 2 * art.target.width as u64 * art.target.height as u64
        + detail_count
            * context.width as u64
            * context.height as u64
            * (request.scale as u64).pow(2);
    if display_pixels > MAX_PREVIEW_PIXELS as u64 {
        return Err(ArtError::new("limit_exceeded", "Combined focus image area exceeds the limit. Reduce region, context_padding, scale or optional outputs")
            .detail(json!({"display_pixels":display_pixels,"max_preview_pixels":MAX_PREVIEW_PIXELS})));
    }
    if request.include_indices && region.width as u64 * region.height as u64 > 4096 {
        return Err(ArtError::new(
            "response_too_large",
            "Request indices for a region of at most 4096 pixels, or set include_indices=false",
        )
        .detail(json!({"region":region,"max_indices_reply":4096})));
    }
    let mut usage = vec![0usize; art.target.palette.len()];
    let rows: Vec<Vec<u16>> = art.indices[region.y as usize..(region.y + region.height) as usize]
        .iter()
        .map(|row| {
            let slice = &row[region.x as usize..(region.x + region.width) as usize];
            for index in slice {
                usage[*index as usize] += 1;
            }
            if request.include_indices {
                slice.to_vec()
            } else {
                vec![]
            }
        })
        .collect();
    let mut images = Vec::new();
    let mut views = Vec::new();
    let overview = render_raster(art, None, 1, &Background::Checkerboard, false)?;
    push_view(
        &mut images,
        &mut views,
        &overview,
        json!({"role":"overview","art_id":art_id,"region":art.target.bounds(),"scale":1,"grid":false} ),
    )?;
    let mut locator = overview;
    outline(&mut locator, context, [51, 136, 238, 255]);
    outline(&mut locator, region, [213, 128, 37, 255]);
    push_view(
        &mut images,
        &mut views,
        &locator,
        json!({"role":"locator","art_id":art_id,"region":art.target.bounds(),"scale":1,"grid":false,"overlays":[{"role":"context","region":context,"rgb":"#3388EE"},{"role":"focus","region":region,"rgb":"#D58025"}]}),
    )?;
    for grid in [false, true] {
        if grid && !request.grid {
            continue;
        }
        let detail = render_raster(
            art,
            Some(context),
            request.scale,
            &Background::Checkerboard,
            grid,
        )?;
        push_view(
            &mut images,
            &mut views,
            &detail,
            json!({"role":if grid {"detail_grid"} else {"detail"},"art_id":art_id,"region":context,"scale":request.scale,"grid":grid}),
        )?;
    }
    let comparison = if let Some(before) = baseline {
        let (context_diff, mask) = compare_raster(before, art, Some(context))?;
        let (focus_diff, _) = compare_raster(before, art, Some(region))?;
        let before_image = render_raster(
            before,
            Some(context),
            request.scale,
            &Background::Checkerboard,
            false,
        )?;
        push_view(
            &mut images,
            &mut views,
            &before_image,
            json!({"role":"before_detail","art_id":before.id()?,"region":context,"scale":request.scale,"grid":false}),
        )?;
        push_view(
            &mut images,
            &mut views,
            &magnify(&mask, request.scale),
            json!({"role":"difference","art_id":art_id,"before_art_id":before.id()?,"region":context,"scale":request.scale,"grid":false}),
        )?;
        Some(json!({"focus":focus_diff,"context":context_diff}))
    } else {
        None
    };
    if let (Some(input), Some(source)) = (&request.reference, reference) {
        let r = input.region;
        let mut crop = Raster {
            width: r.width,
            height: r.height,
            rgba: Vec::with_capacity(r.width as usize * r.height as usize * 4),
        };
        for y in r.y..r.y + r.height {
            let offset = (y as usize * source.width as usize + r.x as usize) * 4;
            crop.rgba
                .extend_from_slice(&source.rgba[offset..offset + r.width as usize * 4]);
        }
        push_view(
            &mut images,
            &mut views,
            &magnify(&crop, input.scale),
            json!({"role":"reference_detail","source_hash":input.source_hash,"coordinate_system":"source_pixel_top_left_xy","region":r,"scale":input.scale,"grid":false}),
        )?;
    }
    Ok(FocusOutput {
        data: json!({"art_id":art_id,"target_hash":hash_json(&art.target)?,"focus_region":region,"context_region":context,"context_padding":padding,"context_clipped":clipped,
            "coordinate_system":"pixel_top_left_xy","scale":request.scale,"views":views,"display_pixels":display_pixels,
            "palette":art.target.palette,"transparent_index":art.target.transparent_index,"allowed_indices":art.target.allowed_indices,
            "focus_usage":usage,"indices":if request.include_indices {Some(rows)} else {None},"comparison":comparison,
            "references":art.references,"implementation":implementation_id()}),
        images,
    })
}

fn push_view(
    images: &mut Vec<Vec<u8>>,
    views: &mut Vec<Value>,
    raster: &Raster,
    mut view: Value,
) -> ArtResult<()> {
    if view.get("coordinate_system").is_none() {
        view["coordinate_system"] = json!("pixel_top_left_xy");
    }
    view["image_index"] = json!(images.len());
    view["display_width"] = json!(raster.width);
    view["display_height"] = json!(raster.height);
    images.push(encode_png(raster)?);
    views.push(view);
    Ok(())
}

fn outline(raster: &mut Raster, region: Rect, color: [u8; 4]) {
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            if x == region.x
                || x == region.x + region.width - 1
                || y == region.y
                || y == region.y + region.height - 1
            {
                let offset = (y as usize * raster.width as usize + x as usize) * 4;
                raster.rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
}

pub(crate) fn magnify(raster: &Raster, scale: u32) -> Raster {
    let mut result = Raster {
        width: raster.width * scale,
        height: raster.height * scale,
        rgba: Vec::with_capacity((raster.width * raster.height * scale * scale * 4) as usize),
    };
    for y in 0..result.height {
        for x in 0..result.width {
            let offset = ((y / scale) as usize * raster.width as usize + (x / scale) as usize) * 4;
            result
                .rgba
                .extend_from_slice(&raster.rgba[offset..offset + 4]);
        }
    }
    result
}
