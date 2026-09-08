#![cfg(feature = "native")]
use dotmend::{art::*, pixels::*, workspace::Workspace};
use serde_json::{Value, json};
use std::fs;
fn target(name: &str) -> Value {
    json!({"resource_id":name,"width":8,"height":8,"palette":["#000000","#000000","#BB4422","#BB4422","#FFFFFF"],"transparent_index":0,"allowed_indices":[0,1,2,3,4],"constraints_ref":null,"requirements":[]})
}
fn all() -> Value {
    json!({"x":0,"y":0,"width":8,"height":8})
}
fn call(w: &mut Workspace, name: &str, args: Value) -> Value {
    w.call(name, args).unwrap().data
}
fn create(w: &mut Workspace, name: &str, rows: Value) -> String {
    call(
        w,
        "create_art",
        json!({"target":target(name),"initial":{"kind":"indices","rows":rows}}),
    )["art_id"]
        .as_str()
        .unwrap()
        .into()
}
fn blank(w: &mut Workspace, name: &str) -> String {
    create(w, name, json!(vec![vec![0; 8]; 8]))
}
fn sel(w: &mut Workspace, base: &str, selector: Value) -> String {
    call(
        w,
        "create_selection",
        json!({"art_id":base,"region":all(),"selector":selector}),
    )["selection_id"]
        .as_str()
        .unwrap()
        .into()
}
fn request(w: &mut Workspace, base: &str, protected: &[String]) -> String {
    call(w,"request_edit",json!({"base_art_id":base,"write_region":all(),"instruction":"Refine the fringe and preserve the face","protected_selection_ids":protected}))["request_id"].as_str().unwrap().into()
}
fn inspect(w: &mut Workspace, id: &str) -> Value {
    call(w, "inspect_edit_request", json!({"request_id":id}))
}
fn count(w: &mut Workspace) -> usize {
    call(w, "list_art", json!({"limit":100}))["arts"]
        .as_array()
        .unwrap()
        .len()
}
fn pixel(x: u32, y: u32, index: u16) -> Value {
    json!({"kind":"set_pixels","pixels":[{"x":x,"y":y,"index":index}]})
}
fn edit(w: &mut Workspace, id: &str, req: Option<&str>, ops: Value) -> String {
    call(
        w,
        "edit_art",
        json!({"art_id":id,"request_id":req,"write_region":all(),"operations":ops}),
    )["art_id"]
        .as_str()
        .unwrap()
        .into()
}

#[test]
fn geometry_diagnoses_exact_violations_and_can_be_repaired_without_changing_conditions() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let mut t = target("actor");
    t["requirements"] = json!([
        {"id":"silhouette","kind":"opaque_bounds","parameters":{"region":{"x":2,"y":2,"width":4,"height":6}},"source_ref":null},
        {"id":"feet","kind":"opaque_bottom","parameters":{"y":7},"source_ref":null}]);
    let base = call(
        &mut w,
        "create_art",
        json!({"target":t,"initial":{"kind":"fill","index":0}}),
    )["art_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let empty = call(&mut w, "validate_art", json!({"art_id":base}));
    assert_eq!(empty["checks"][1]["status"], "pass");
    assert_eq!(empty["checks"][2]["actual"]["y"], Value::Null);
    assert_eq!(empty["checks"][2]["locations"][0]["role"], "expected");
    let drawn = edit(
        &mut w,
        &base,
        None,
        json!([pixel(0, 0, 1), pixel(7, 3, 2), pixel(3, 4, 3)]),
    );
    let report = call(&mut w, "validate_art", json!({"art_id":drawn}));
    assert_eq!(report["status"], "fail");
    assert_eq!(report["checks"][1]["actual"]["outside_count"], 2);
    assert_eq!(report["checks"][2]["actual"]["y"], 4);
    for check in report["checks"].as_array().unwrap() {
        for loc in check["locations"].as_array().unwrap() {
            let focus = call(
                &mut w,
                "focus_art",
                json!({"art_id":drawn,"region":loc["region"],"context_padding":0,"scale":2}),
            );
            assert_eq!(focus["focus_region"], loc["region"]);
        }
    }
    assert_eq!(
        w.call("export_art", json!({"art_id":drawn}))
            .err()
            .unwrap()
            .code,
        "validation_blocked"
    );
    let repaired = edit(
        &mut w,
        &drawn,
        None,
        json!([pixel(0, 0, 0), pixel(7, 3, 0), pixel(3, 7, 2)]),
    );
    assert_eq!(
        w.load(&repaired).unwrap().target,
        w.load(&base).unwrap().target
    );
    assert_eq!(
        call(&mut w, "validate_art", json!({"art_id":repaired}))["status"],
        "pass"
    );
    let bundle = call(&mut w, "export_art", json!({"art_id":repaired}));
    let imported = call(
        &mut w,
        "create_art",
        json!({"bundle_path":bundle["bundle_path"]}),
    );
    assert_eq!(
        w.load(imported["art_id"].as_str().unwrap())
            .unwrap()
            .indices,
        w.load(&repaired).unwrap().indices
    );
}
#[test]
fn malformed_geometry_is_an_input_error_and_duplicate_rgb_is_still_opaque() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    for (kind, parameters) in [
        ("opaque_bottom", json!({"y":8})),
        ("opaque_bottom", json!({"y":-1})),
        ("opaque_bottom", json!({"y":1,"extra":true})),
        (
            "opaque_bounds",
            json!({"region":{"x":0,"y":0,"width":0,"height":1}}),
        ),
        (
            "opaque_bounds",
            json!({"region":{"x":u32::MAX,"y":0,"width":2,"height":1}}),
        ),
    ] {
        let mut t = target("bad");
        t["requirements"] =
            json!([{"id":"placement","kind":kind,"parameters":parameters,"source_ref":null}]);
        let error = w
            .call(
                "create_art",
                json!({"target":t,"initial":{"kind":"fill","index":0}}),
            )
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid_input");
        assert_eq!(error.details["requirement_id"], "placement");
    }
    let mut t = target("opaque");
    t["transparent_index"] = Value::Null;
    t["requirements"] =
        json!([{"id":"feet","kind":"opaque_bottom","parameters":{"y":7},"source_ref":null}]);
    let id = call(
        &mut w,
        "create_art",
        json!({"target":t,"initial":{"kind":"fill","index":0}}),
    )["art_id"]
        .clone();
    let report = call(&mut w, "validate_art", json!({"art_id":id}));
    assert_eq!(report["checks"][1]["actual"]["opaque_count"], 64);
    assert_eq!(report["status"], "pass");
}
#[test]
fn selections_freeze_index_connectivity_and_survive_restart() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let mut rows = vec![vec![0; 8]; 8];
    rows[1][1] = 2;
    rows[2][2] = 2;
    rows[1][2] = 3;
    let base = create(&mut w, "mask", json!(rows));
    let four = sel(
        &mut w,
        &base,
        json!({"kind":"connected_indices","seed":{"x":1,"y":1},"indices":[2],"connectivity":4}),
    );
    let eight = sel(
        &mut w,
        &base,
        json!({"kind":"connected_indices","seed":{"x":1,"y":1},"indices":[2],"connectivity":8}),
    );
    let read = |w: &Workspace, id: &str| {
        serde_json::from_slice::<Value>(
            &w.resource(&format!("dotmend://selections/{id}")).unwrap().0,
        )
        .unwrap()
    };
    assert_eq!(read(&w, &four)["pixel_count"], 1);
    assert_eq!(read(&w, &eight)["pixel_count"], 2);
    assert_eq!(
        w.resource(&format!("retro-art://selections/{eight}"))
            .unwrap(),
        w.resource(&format!("dotmend://selections/{eight}"))
            .unwrap()
    );
    let req=call(&mut w,"request_edit",json!({"base_art_id":base,"write_region":all(),"instruction":"Edit connected pixels","write_selection_id":eight}))["request_id"].as_str().unwrap().to_owned();
    let changed = edit(&mut w, &base, Some(&req), json!([pixel(1, 1, 4)]));
    let next = edit(&mut w, &changed, Some(&req), json!([pixel(2, 2, 1)]));
    assert_eq!(w.load(&next).unwrap().indices[1][2], 3);
    let copy = read(&w, &eight);
    drop(w);
    let mut w = Workspace::open(temp.path()).unwrap();
    assert_eq!(read(&w, &eight), copy);
    assert_eq!(inspect(&mut w, &req)["selections"][0]["pixel_count"], 2);
    let other = blank(&mut w, "other");
    assert_eq!(w.call("request_edit",json!({"base_art_id":other,"write_region":all(),"instruction":"wrong","write_selection_id":eight})).err().unwrap().code,"selection_mismatch");
}
#[test]
fn every_write_operation_enforces_protection_atomically_including_same_index_and_restore() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let base = blank(&mut w, "guard");
    let protected = sel(
        &mut w,
        &base,
        json!({"kind":"pixels","points":[{"x":2,"y":2}]}),
    );
    let req = request(&mut w, &base, &[protected]);
    let solid = edit(&mut w, &base, None, json!([pixel(2, 2, 2)]));
    let attempts = vec![
        json!([pixel(2, 2, 0)]),
        json!([{"kind":"paint_rows","x":2,"y":2,"rows":[[0]]}]),
        json!([{"kind":"fill_rect","rect":{"x":2,"y":2,"width":1,"height":1},"index":0}]),
        json!([{"kind":"replace_index","rect":{"x":2,"y":2,"width":1,"height":1},"from_index":0,"to_index":0}]),
        json!([{"kind":"paste","source_art_id":solid,"source_rect":{"x":2,"y":2,"width":1,"height":1},"x":2,"y":2,"flip_x":false,"flip_y":false,"mode":"over"}]),
        json!([{"kind":"paste","source_art_id":base,"source_rect":{"x":2,"y":2,"width":1,"height":1},"x":2,"y":2,"flip_x":false,"flip_y":false,"mode":"replace"}]),
        json!([pixel(0, 0, 2), pixel(2, 2, 2), pixel(2, 2, 0)]),
    ];
    let before = count(&mut w);
    for operations in attempts {
        let error=w.call("edit_art",json!({"art_id":base,"request_id":req,"write_region":all(),"operations":operations})).err().unwrap();
        assert_eq!(error.code, "protected_pixel");
        assert_eq!(error.details["cause"]["x"], 2);
        assert_eq!(count(&mut w), before);
    }
    let no_write = json!([{"kind":"paint_rows","x":2,"y":2,"rows":[[null]]},{"kind":"replace_index","rect":{"x":2,"y":2,"width":1,"height":1},"from_index":2,"to_index":3},{"kind":"paste","source_art_id":base,"source_rect":{"x":2,"y":2,"width":1,"height":1},"x":2,"y":2,"flip_x":false,"flip_y":false,"mode":"over"}]);
    assert_eq!(edit(&mut w, &base, Some(&req), no_write), base);
    assert_eq!(
        w.call(
            "submit_edit_result",
            json!({"request_id":req,"result_art_id":solid,"notes":"bypass"})
        )
        .err()
        .unwrap()
        .code,
        "protected_pixel"
    );
    assert_eq!(w.call("edit_art",json!({"art_id":solid,"request_id":req,"write_region":all(),"operations":[pixel(0,0,2)]})).err().unwrap().code,"protected_pixel");
}
#[test]
fn protected_set_plans_bind_every_request_and_a_failure_saves_no_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let a = blank(&mut w, "frame.a");
    let b = blank(&mut w, "frame.b");
    let ma = sel(
        &mut w,
        &a,
        json!({"kind":"pixels","points":[{"x":2,"y":2}]}),
    );
    let mb = sel(
        &mut w,
        &b,
        json!({"kind":"pixels","points":[{"x":1,"y":1}]}),
    );
    let ra = request(&mut w, &a, &[ma]);
    let rb = request(&mut w, &b, &[mb]);
    let mut args = json!({"template_art_id":a,"art_ids":[a,b],"request_bindings":[{"art_id":a,"request_id":ra},{"art_id":b,"request_id":rb}],"write_region":all(),"operations":[pixel(1,1,2)],"preview":true});
    let before = count(&mut w);
    let preview = call(&mut w, "edit_art_set", args.clone());
    assert_eq!(preview["compatible"], false);
    args["preview"] = json!(false);
    assert!(w.call("edit_art_set", args.clone()).is_err());
    assert_eq!(count(&mut w), before);
    args["preview"] = json!(true);
    args["operations"] = json!([pixel(0, 0, 2)]);
    let preview = call(&mut w, "edit_art_set", args.clone());
    assert_eq!(preview["compatible"], true);
    args["expected_plan_hash"] = preview["plan_hash"].clone();
    args["preview"] = json!(false);
    let applied = call(&mut w, "edit_art_set", args);
    assert_eq!(applied["candidates_saved"], true);
    for result in applied["results"].as_array().unwrap() {
        assert_eq!(
            w.load(result["art_id"].as_str().unwrap()).unwrap().indices[0][0],
            2
        );
    }
    assert_eq!(w.load(&a).unwrap().indices[0][0], 0);
    assert_eq!(w.load(&b).unwrap().indices[0][0], 0);
}
#[test]
fn review_feedback_followups_and_exact_retries_persist_without_reverting_newer_decisions() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let base = blank(&mut w, "review");
    let req = request(&mut w, &base, &[]);
    let result = edit(&mut w, &base, Some(&req), json!([pixel(1, 1, 2)]));
    let submission = json!({"request_id":req,"result_art_id":result,"notes":"Refined the fringe"});
    call(&mut w, "submit_edit_result", submission.clone());
    call(&mut w, "submit_edit_result", submission.clone());
    let review = json!({"request_id":req,"result_art_id":result,"decision":"changes_requested","notes":"Keep the eyes","regions":[{"region":{"x":1,"y":1,"width":1,"height":1},"comment":"Brighten only this pixel"}],"expected_review_id":null,"follow_up":{"base_art_id":result,"write_region":all(),"instruction":"Brighten the hair tips","preserve_notes":"Keep the eyes unchanged"}});
    let first = w
        .review_result(serde_json::from_value(review.clone()).unwrap())
        .unwrap()
        .data;
    let next_id = first["review"]["follow_up_request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let next = inspect(&mut w, &next_id);
    assert_eq!(next["request"]["previous_request_id"], req);
    assert_eq!(next["request"]["base_art_id"], result);
    let accepted = json!({"request_id":req,"result_art_id":result,"decision":"accepted","notes":"Accept the current result","regions":[],"expected_review_id":first["current_review_id"]});
    let second = w
        .review_result(serde_json::from_value(accepted).unwrap())
        .unwrap()
        .data;
    let retry = w
        .review_result(serde_json::from_value(review.clone()).unwrap())
        .unwrap()
        .data;
    assert_eq!(retry["review"], first["review"]);
    assert_eq!(retry["current_review_id"], second["current_review_id"]);
    assert_eq!(retry["human_review"], "accepted");
    let pinned = inspect(&mut w, &next_id);
    assert_eq!(
        pinned["previous_review"]["review_id"],
        first["current_review_id"]
    );
    assert_eq!(pinned["previous_review"]["input"]["notes"], "Keep the eyes");
    let repeated = call(&mut w, "submit_edit_result", submission.clone());
    assert_eq!(repeated["human_review"], "accepted");
    let mut conflict = review;
    conflict["notes"] = json!("Stale view");
    assert_eq!(
        w.review_result(serde_json::from_value(conflict).unwrap())
            .err()
            .unwrap()
            .code,
        "review_conflict"
    );
    let mut different = submission;
    different["notes"] = json!("Different description");
    assert_eq!(
        w.call("submit_edit_result", different).err().unwrap().code,
        "submission_conflict"
    );
    let missing = json!({"request_id":req,"result_art_id":result,"decision":"accepted","notes":"","regions":[]});
    assert!(serde_json::from_value::<dotmend::workflow::ReviewEditResult>(missing).is_err());
    drop(w);
    let mut w = Workspace::open(temp.path()).unwrap();
    let resumed = inspect(&mut w, &req);
    assert_eq!(resumed["human_review"], "accepted");
    assert_eq!(resumed["reviews"].as_array().unwrap().len(), 2);
    assert_eq!(resumed["follow_up_requests"].as_array().unwrap().len(), 1);
    let page = call(
        &mut w,
        "inspect_edit_request",
        json!({"request_id":req,"limit":1}),
    );
    assert!(page["next_review_cursor"].is_i64());
    let more = call(
        &mut w,
        "inspect_edit_request",
        json!({"request_id":req,"limit":1,"review_cursor":page["next_review_cursor"]}),
    );
    assert_eq!(more["reviews"][0]["review_id"], second["current_review_id"]);
}
#[test]
fn failed_review_transaction_rolls_back_its_followup_and_allows_retry() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let base = blank(&mut w, "rollback");
    let req = request(&mut w, &base, &[]);
    call(
        &mut w,
        "submit_edit_result",
        json!({"request_id":req,"result_art_id":base,"notes":""}),
    );
    let db = rusqlite::Connection::open(temp.path().join(".retro-art/art.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_review BEFORE INSERT ON edit_reviews BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    let input = json!({"request_id":req,"result_art_id":base,"decision":"changes_requested","notes":"Try again","regions":[],"expected_review_id":null,"follow_up":{"base_art_id":base,"write_region":all(),"instruction":"Follow-up"}});
    assert_eq!(
        w.review_result(serde_json::from_value(input.clone()).unwrap())
            .err()
            .unwrap()
            .code,
        "storage_error"
    );
    assert_eq!(inspect(&mut w, &req)["human_review"], "pending");
    assert!(
        inspect(&mut w, &req)["follow_up_requests"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    db.execute_batch("DROP TRIGGER reject_review").unwrap();
    w.review_result(serde_json::from_value(input).unwrap())
        .unwrap();
}
#[test]
fn reference_focus_uses_source_coordinates_and_preserves_rgba_with_aggregate_limits() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let base = blank(&mut w, "reference");
    let source = Raster {
        width: 12,
        height: 10,
        rgba: (0..120)
            .flat_map(|i| [(i % 12) as u8, (i / 12) as u8, 55, 128])
            .collect(),
    };
    let bytes = encode_png(&source).unwrap();
    fs::write(temp.path().join("source.png"), &bytes).unwrap();
    let attached = call(
        &mut w,
        "attach_reference",
        json!({"art_id":base,"source_path":"source.png","role":"reference","label":"Reference at original size"}),
    );
    let id = attached["art_id"].as_str().unwrap();
    let hash = digest(&bytes);
    let before = count(&mut w);
    let args = json!({"art_id":id,"region":{"x":1,"y":2,"width":2,"height":2},"context_padding":0,"scale":4,"reference":{"source_hash":hash,"region":{"x":8,"y":5,"width":3,"height":2},"scale":2}});
    let focused = w.call("focus_art", args.clone()).unwrap();
    let view = focused.data["views"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["role"] == "reference_detail")
        .unwrap();
    assert_eq!(view["coordinate_system"], "source_pixel_top_left_xy");
    assert!(view.get("art_id").is_none());
    let image =
        decode_png(&focused.images[view["image_index"].as_u64().unwrap() as usize]).unwrap();
    assert_eq!((image.width, image.height), (6, 4));
    assert_eq!(&image.rgba[..4], &[8, 5, 55, 128]);
    assert_eq!(count(&mut w), before);
    let mut bad = args.clone();
    bad["reference"]["region"]["x"] = json!(12);
    assert!(w.call("focus_art", bad).is_err());
    let mut unlinked = args;
    unlinked["art_id"] = json!(base);
    assert!(w.call("focus_art", unlinked).is_err());
}
#[test]
fn frame_views_align_declared_anchors_and_leave_unknown_timing_unplayable() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let base = blank(&mut w, "frame");
    let a = edit(&mut w, &base, None, json!([pixel(2, 7, 2)]));
    let b = edit(&mut w, &base, None, json!([pixel(4, 6, 4)]));
    let before = count(&mut w);
    let mut args = json!({"frames":[{"art_id":a,"label":"Idle","anchor":{"x":2,"y":7},"duration_ms":100},{"art_id":b,"label":"Walk","anchor":{"x":4,"y":6}}],"scale":2,"view":"frames"});
    let output = w.call("render_art_set", args.clone()).unwrap();
    assert_eq!(output.data["playable"], false);
    for (i, f) in output.data["frames"].as_array().unwrap().iter().enumerate() {
        let raster = decode_png(&output.images[i]).unwrap();
        let x = f["display_region"]["x"].as_u64().unwrap() + f["anchor"]["x"].as_u64().unwrap() * 2;
        let y = f["display_region"]["y"].as_u64().unwrap() + f["anchor"]["y"].as_u64().unwrap() * 2;
        assert_eq!((x, y), (8, 14));
        let offset = (y as usize * raster.width as usize + x as usize) * 4;
        assert_eq!(raster.rgba[offset + 3], 255);
    }
    args["frames"][1]["duration_ms"] = json!(120);
    args["view"] = json!("sheet");
    assert_eq!(
        call(&mut w, "render_art_set", args.clone())["playable"],
        true
    );
    assert_eq!(count(&mut w), before);
    args["frames"][1].as_object_mut().unwrap().remove("anchor");
    assert!(w.call("render_art_set", args).is_err());
    let huge = json!({"frames":[{"art_id":a,"label":"a","anchor":{"x":0,"y":0}},{"art_id":b,"label":"b","anchor":{"x":8,"y":8}}],"scale":64,"view":"sheet"});
    assert_eq!(
        w.call("render_art_set", huge).err().unwrap().code,
        "limit_exceeded"
    );
}
#[test]
fn opening_legacy_storage_preserves_candidate_and_request_payloads_and_ids() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let base = blank(&mut w, "legacy");
    let req = request(&mut w, &base, &[]);
    drop(w);
    let db = rusqlite::Connection::open(temp.path().join(".retro-art/art.sqlite")).unwrap();
    let bytes: Vec<u8> = db
        .query_row(
            "SELECT payload FROM edit_requests WHERE id=?1",
            [&req],
            |r| r.get(0),
        )
        .unwrap();
    let art_bytes: Vec<u8> = db
        .query_row("SELECT payload FROM arts WHERE id=?1", [&base], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute_batch("DROP TABLE edit_reviews; DROP TABLE selections;")
        .unwrap();
    drop(db);
    let mut w = Workspace::open(temp.path()).unwrap();
    assert_eq!(inspect(&mut w, &req)["request"]["base_art_id"], base);
    assert_eq!(w.load(&base).unwrap().id().unwrap(), base);
    assert_eq!(request(&mut w, &base, &[]), req);
    let db = rusqlite::Connection::open(temp.path().join(".retro-art/art.sqlite")).unwrap();
    let after: Vec<u8> = db
        .query_row(
            "SELECT payload FROM edit_requests WHERE id=?1",
            [&req],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, bytes);
    let after: Vec<u8> = db
        .query_row("SELECT payload FROM arts WHERE id=?1", [&base], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(after, art_bytes);
}

#[test]
fn simultaneous_reviewers_preserve_one_decision_and_report_the_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(temp.path()).unwrap();
    let base = blank(&mut w, "concurrent.review");
    let req = request(&mut w, &base, &[]);
    call(
        &mut w,
        "submit_edit_result",
        json!({"request_id":req,"result_art_id":base,"notes":""}),
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = vec![];
    for notes in ["First view", "Second view"] {
        let root = temp.path().to_owned();
        let barrier = barrier.clone();
        let input = json!({"request_id":req,"result_art_id":base,"decision":"accepted","notes":notes,"regions":[],"expected_review_id":null});
        workers.push(std::thread::spawn(move || {
            let mut w = Workspace::open(root).unwrap();
            barrier.wait();
            w.review_result(serde_json::from_value(input).unwrap())
                .map(|o| o.data)
                .map_err(|e| e.code)
        }));
    }
    let results = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r,Err(e) if e=="review_conflict"))
            .count(),
        1
    );
    assert_eq!(
        inspect(&mut w, &req)["reviews"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn public_schemas_require_nullable_fields_without_removing_null_values() {
    use dotmend::{outputs::output_schema, requests::input_schema, workflow::ReviewEditResult};
    fn permits_null(schema: &Value) -> bool {
        schema["type"] == "null"
            || schema["type"]
                .as_array()
                .is_some_and(|types| types.contains(&json!("null")))
            || schema["anyOf"]
                .as_array()
                .is_some_and(|choices| choices.iter().any(permits_null))
    }
    fn required_nullable(schema: &Value, field: &str) {
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!(field))
        );
        assert!(
            permits_null(&schema["properties"][field]),
            "{field}: {schema}"
        );
    }
    let target = input_schema::<Target>();
    required_nullable(&target, "transparent_index");
    required_nullable(&target, "constraints_ref");
    required_nullable(&input_schema::<Requirement>(), "source_ref");
    let review = input_schema::<ReviewEditResult>();
    required_nullable(&review, "expected_review_id");
    assert!(
        !review["required"]
            .as_array()
            .unwrap()
            .contains(&json!("follow_up"))
    );
    required_nullable(&output_schema("list_art")["oneOf"][0], "next_cursor");
    required_nullable(
        &output_schema("inspect_edit_request")["oneOf"][0],
        "latest_review",
    );
}
