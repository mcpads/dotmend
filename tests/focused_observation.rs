use dotmend::{
    art::*,
    focus::{FocusOutput, focus},
    pixels::{Raster, decode_png},
    requests::FocusArt,
};
use serde_json::{Value, json};

fn pattern() -> Art {
    Art {
        target: Target {
            resource_id: "hero.front".into(),
            width: 8,
            height: 6,
            palette: vec!["#000000", "#000000", "#FF0000", "#FF0000", "#FFFFFF"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            transparent_index: Some(0),
            allowed_indices: vec![0, 1, 2, 3, 4],
            constraints_ref: None,
            requirements: vec![],
        },
        indices: (0..6)
            .map(|y| (0..8).map(|x| (1 + (x + y * 3) % 4) as u16).collect())
            .collect(),
        context: ArtContext::default(),
        references: vec![],
        parents: vec![],
        provenance: json!({}),
    }
}
fn request(art: &Art, region: Rect, padding: u32, scale: u32) -> FocusArt {
    FocusArt {
        reference: None,
        art_id: art.id().unwrap(),
        region,
        context_padding: padding,
        scale,
        grid: false,
        include_indices: true,
        compare_to_art_id: None,
    }
}
fn view(output: &FocusOutput, role: &str) -> (Value, Raster) {
    let view = output.data["views"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["role"] == role)
        .unwrap();
    let image = decode_png(&output.images[view["image_index"].as_u64().unwrap() as usize]).unwrap();
    assert_eq!(view["display_width"], image.width);
    assert_eq!(view["display_height"], image.height);
    (view.clone(), image)
}
fn pixel(raster: &Raster, x: u32, y: u32) -> &[u8] {
    let offset = (y as usize * raster.width as usize + x as usize) * 4;
    &raster.rgba[offset..offset + 4]
}

#[test]
fn focused_views_preserve_pixel_coordinates_at_the_center_and_canvas_edges() {
    let art = pattern();
    let snapshot = encode_json(&art).unwrap();
    let cases = [
        (
            Rect {
                x: 3,
                y: 2,
                width: 2,
                height: 2,
            },
            Rect {
                x: 1,
                y: 0,
                width: 6,
                height: 6,
            },
            json!({"left":0,"top":0,"right":0,"bottom":0}),
        ),
        (
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            Rect {
                x: 0,
                y: 0,
                width: 3,
                height: 3,
            },
            json!({"left":2,"top":2,"right":0,"bottom":0}),
        ),
        (
            Rect {
                x: 7,
                y: 5,
                width: 1,
                height: 1,
            },
            Rect {
                x: 5,
                y: 3,
                width: 3,
                height: 3,
            },
            json!({"left":0,"top":0,"right":2,"bottom":2}),
        ),
    ];
    for (region, context, clipped) in cases {
        for scale in [1, 4, 9] {
            let output = focus(&art, None, &request(&art, region, 2, scale)).unwrap();
            assert_eq!(output.data["context_region"], json!(context));
            assert_eq!(output.data["context_clipped"], clipped);
            assert_eq!(output.data["focus_region"], json!(region));
            let expected: Vec<Vec<u16>> = art.indices
                [region.y as usize..(region.y + region.height) as usize]
                .iter()
                .map(|r| r[region.x as usize..(region.x + region.width) as usize].to_vec())
                .collect();
            assert_eq!(output.data["indices"], json!(expected));
            let mut counts = vec![0; art.target.palette.len()];
            for index in expected.iter().flatten() {
                counts[*index as usize] += 1;
            }
            assert_eq!(output.data["focus_usage"], json!(counts));
            let (detail, image) = view(&output, "detail");
            assert_eq!(detail["region"], json!(context));
            assert_eq!(
                (image.width, image.height),
                (context.width * scale, context.height * scale)
            );
            let (_, overview) = view(&output, "overview");
            for y in 0..image.height {
                for x in 0..image.width {
                    let source_x = context.x + x / scale;
                    let source_y = context.y + y / scale;
                    let color = parse_rgb(
                        &art.target.palette
                            [art.indices[source_y as usize][source_x as usize] as usize],
                    )
                    .unwrap();
                    assert_eq!(pixel(&image, x, y), &[color[0], color[1], color[2], 255]);
                    assert_eq!(pixel(&overview, source_x, source_y), pixel(&image, x, y));
                }
            }
            let (locator_meta, locator) = view(&output, "locator");
            let overlay = locator_meta["overlays"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["role"] == "focus")
                .unwrap();
            assert_eq!(overlay["region"], json!(region));
            let color = parse_rgb(overlay["rgb"].as_str().unwrap()).unwrap();
            assert_eq!(
                pixel(&locator, region.x, region.y),
                &[color[0], color[1], color[2], 255]
            );
            assert_eq!(output.data["comparison"], Value::Null);
        }
    }
    assert_eq!(encode_json(&art).unwrap(), snapshot);
}

#[test]
fn focus_comparison_separates_local_and_context_changes_and_duplicate_colors() {
    let mut before = pattern();
    before.indices[2][2] = 2;
    before.indices[1][1] = 1;
    let mut after = before.clone();
    after.indices[2][2] = 3;
    after.indices[1][1] = 4;
    let mut args = request(
        &after,
        Rect {
            x: 2,
            y: 2,
            width: 2,
            height: 2,
        },
        1,
        4,
    );
    args.compare_to_art_id = Some(before.id().unwrap());
    args.grid = true;
    let output = focus(&after, Some(&before), &args).unwrap();
    assert_eq!(output.data["comparison"]["focus"]["changed_pixels"], 1);
    assert_eq!(
        output.data["comparison"]["focus"]["visual_changed_pixels"],
        0
    );
    assert_eq!(output.data["comparison"]["context"]["changed_pixels"], 2);
    assert_eq!(
        output.data["comparison"]["context"]["visual_changed_pixels"],
        1
    );
    let (detail_meta, plain) = view(&output, "detail");
    let (before_meta, _) = view(&output, "before_detail");
    let (difference_meta, mask) = view(&output, "difference");
    assert_eq!(detail_meta["region"], before_meta["region"]);
    assert_eq!(detail_meta["region"], difference_meta["region"]);
    assert_eq!(pixel(&mask, 0, 0), &[255, 80, 140, 255]);
    assert_eq!(pixel(&mask, 4, 4), &[255, 80, 140, 255]);
    assert_eq!(pixel(&mask, 8, 8), &[0, 0, 0, 0]);
    let (_, grid) = view(&output, "detail_grid");
    assert_ne!(pixel(&plain, 0, 0), pixel(&grid, 0, 0));
    assert_eq!(pixel(&plain, 1, 1), pixel(&grid, 1, 1));
    before.indices[2][2] = 0;
    after.indices[2][2] = 1;
    args.art_id = after.id().unwrap();
    args.compare_to_art_id = Some(before.id().unwrap());
    let transparent = focus(&after, Some(&before), &args).unwrap();
    assert_eq!(
        transparent.data["comparison"]["focus"]["visual_changed_pixels"],
        1
    );
}

#[test]
fn focus_rejects_invalid_candidates_and_aggregate_limits_without_partial_results() {
    let art = pattern();
    let region = Rect {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let mut args = request(&art, region, u32::MAX, 1);
    let output = focus(&art, None, &args).unwrap();
    assert_eq!(output.data["context_region"], json!(art.target.bounds()));
    assert_eq!(output.data["context_clipped"]["left"], u32::MAX);
    args.region.x = 8;
    assert_eq!(
        focus(&art, None, &args).err().unwrap().code,
        "out_of_bounds"
    );
    args.region = region;
    for scale in [0, 65, u32::MAX] {
        args.scale = scale;
        assert_eq!(
            focus(&art, None, &args).err().unwrap().code,
            "limit_exceeded"
        );
    }
    args.scale = 1;
    args.grid = true;
    assert_eq!(
        focus(&art, None, &args).err().unwrap().code,
        "invalid_input"
    );
    args.grid = false;
    let mut other = art.clone();
    other.target.resource_id = "other".into();
    args.compare_to_art_id = Some(other.id().unwrap());
    assert_eq!(
        focus(&art, Some(&other), &args).err().unwrap().code,
        "target_mismatch"
    );
    assert_eq!(
        focus(&art, None, &args).err().unwrap().code,
        "invalid_input"
    );
    let mut large = art;
    large.target.width = 512;
    large.target.height = 512;
    large.indices = vec![vec![1; 512]; 512];
    args = request(
        &large,
        Rect {
            x: 0,
            y: 0,
            width: 256,
            height: 256,
        },
        0,
        4,
    );
    args.include_indices = false;
    let limit = focus(&large, None, &args).err().unwrap();
    assert_eq!(limit.code, "limit_exceeded");
    assert_eq!(limit.details["display_pixels"], 1572864);
    args = request(
        &large,
        Rect {
            x: 0,
            y: 0,
            width: 65,
            height: 64,
        },
        0,
        1,
    );
    assert_eq!(
        focus(&large, None, &args).err().unwrap().code,
        "response_too_large"
    );
    args.include_indices = false;
    let without_rows = focus(&large, None, &args).unwrap();
    assert_eq!(without_rows.data["indices"], Value::Null);
}
