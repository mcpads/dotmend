#![cfg(feature = "native")]
use dotmend::{presentation::HumanAction, workspace::Workspace};
use serde_json::{Value, json};
fn call(w: &mut Workspace, name: &str, args: Value) -> Value {
    w.call(name, args).unwrap().data
}
fn art(w: &mut Workspace, name: &str) -> String {
    call(w,"create_art",json!({"target":{"resource_id":name,"width":8,"height":8,"palette":["#000000","#000000","#E07440"],"transparent_index":0,"allowed_indices":[0,1,2],"constraints_ref":null,"requirements":[]},"initial":{"kind":"fill","index":0}}))["art_id"].as_str().unwrap().into()
}
fn current(w: &mut Workspace) -> Value {
    call(w, "inspect_presentation", json!({}))["presentation"].clone()
}
fn arguments(w: &mut Workspace, ids: &[String]) -> Value {
    let old = current(w);
    json!({"title":"Refine the hair pixels","items":ids.iter().enumerate().map(|(i,id)|json!({"kind":"art","art_id":id,"label":format!("Image {i}"),"region":{"x":0,"y":0,"width":8,"height":8},"scale":16,"editable":true})).collect::<Vec<_>>(),"expected_presentation_id":old["presentation_id"],"expected_state_id":old["state_id"]})
}
fn show(w: &mut Workspace, ids: &[String]) -> Value {
    let args = arguments(w, ids);
    call(w, "present_art", args)["presentation"].clone()
}
fn command(view: &Value, kind: &str) -> Value {
    json!({"action":kind,"presentation_id":view["presentation_id"],"expected_state_id":view["state_id"]})
}
fn paint(view: &Value, x: u32, y: u32) -> Value {
    let mut input = command(view, "paint");
    input["item_index"] = json!(0);
    input["pixels"] = json!([{"x":x,"y":y,"index":2}]);
    input
}
fn act(w: &mut Workspace, args: Value) -> Value {
    w.human_action(serde_json::from_value::<HumanAction>(args).unwrap())
        .unwrap()
        .data["presentation"]
        .clone()
}
fn count(w: &mut Workspace) -> usize {
    call(w, "list_art", json!({"limit":100}))["arts"]
        .as_array()
        .unwrap()
        .len()
}

#[test]
fn pixel_edits_have_one_undo_and_an_explicit_saved_result_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "actor");
    let initial = show(&mut w, std::slice::from_ref(&base));
    let first = act(&mut w, paint(&initial, 1, 1));
    let first_id = first["state"]["art_ids"][0].clone();
    let second = act(&mut w, paint(&first, 2, 1));
    let previous_state = first["state_id"].clone();
    drop(w);
    let mut w = Workspace::open(dir.path()).unwrap();
    assert_eq!(current(&mut w)["state_id"], second["state_id"]);
    let undone = act(&mut w, command(&second, "undo"));
    assert_eq!(undone["state"]["art_ids"][0], first_id);
    assert_eq!(undone["undo_available"], false);
    let error = w
        .human_action(serde_json::from_value(command(&undone, "undo")).unwrap())
        .err()
        .unwrap();
    assert_eq!(error.code, "invalid_input");
    let saved = act(&mut w, command(&undone, "save"));
    assert_eq!(saved["dirty"], false);
    assert_eq!(saved["saved"]["art_ids"][0], first_id);
    let older = call(
        &mut w,
        "inspect_presentation",
        json!({"state_id":previous_state}),
    )["presentation"]
        .clone();
    assert_eq!(older["is_current"], false);
    assert_eq!(older["state"]["art_ids"][0], first_id);
    drop(w);
    let mut w = Workspace::open(dir.path()).unwrap();
    assert_eq!(current(&mut w)["saved"], saved["saved"]);
    assert_eq!(count(&mut w), 3);
}
#[test]
fn request_protection_is_enforced_on_a_full_picture_without_exposing_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "guarded");
    let selection = call(
        &mut w,
        "create_selection",
        json!({"art_id":base,"region":{"x":1,"y":1,"width":1,"height":1},"selector":{"kind":"rect"}}),
    );
    let request = call(
        &mut w,
        "request_edit",
        json!({"base_art_id":base,"write_region":{"x":0,"y":0,"width":4,"height":4},"instruction":"Preserve the face","protected_selection_ids":[selection["selection_id"]]}),
    );
    let mut args = arguments(&mut w, std::slice::from_ref(&base));
    args["items"][0]["request_id"] = request["request_id"].clone();
    let view = call(&mut w, "present_art", args)["presentation"].clone();
    let before = count(&mut w);
    let mut bad = paint(&view, 0, 0);
    bad["pixels"]
        .as_array_mut()
        .unwrap()
        .push(json!({"x":1,"y":1,"index":0}));
    assert_eq!(
        w.human_action(serde_json::from_value(bad).unwrap())
            .err()
            .unwrap()
            .code,
        "protected_pixel"
    );
    assert_eq!(count(&mut w), before);
    assert_eq!(current(&mut w)["state_id"], view["state_id"]);
    let good = act(&mut w, paint(&view, 2, 2));
    let result = call(
        &mut w,
        "inspect_art",
        json!({"art_id":good["state"]["art_ids"][0],"include_indices":true}),
    );
    assert_eq!(result["indices"][1][1], 0);
    assert_eq!(result["provenance"]["request_id"], request["request_id"]);
    let outside = paint(&good, 7, 7);
    assert_eq!(
        w.human_action(serde_json::from_value(outside).unwrap())
            .err()
            .unwrap()
            .code,
        "out_of_bounds"
    );
}
#[test]
fn stale_actions_and_agent_presentations_cannot_replace_newer_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let a = art(&mut w, "a");
    let b = art(&mut w, "b");
    let view = show(&mut w, std::slice::from_ref(&a));
    let stale_show = arguments(&mut w, std::slice::from_ref(&b));
    let edit = paint(&view, 1, 1);
    let painted = act(&mut w, edit.clone());
    assert_eq!(
        w.call("present_art", stale_show).err().unwrap().code,
        "presentation_conflict"
    );
    assert_eq!(current(&mut w)["state_id"], painted["state_id"]);
    let saved_command = command(&painted, "save");
    let saved = act(&mut w, saved_command.clone());
    let show_b = arguments(&mut w, std::slice::from_ref(&b));
    let second = call(&mut w, "present_art", show_b.clone())["presentation"].clone();
    assert_eq!(
        act(&mut w, edit)["presentation_id"],
        second["presentation_id"]
    );
    assert_eq!(
        act(&mut w, saved_command)["presentation_id"],
        second["presentation_id"]
    );
    assert_eq!(
        w.human_action(serde_json::from_value(paint(&painted, 2, 2)).unwrap())
            .err()
            .unwrap()
            .code,
        "presentation_conflict"
    );
    let show_a = show(&mut w, std::slice::from_ref(&a));
    let retry = call(&mut w, "present_art", show_b)["presentation"].clone();
    assert_eq!(retry["is_current"], false);
    assert_eq!(
        current(&mut w)["presentation_id"],
        show_a["presentation_id"]
    );
    let old = call(
        &mut w,
        "inspect_presentation",
        json!({"presentation_id":view["presentation_id"]}),
    );
    assert_eq!(old["presentation"]["saved"], saved["saved"]);
}
#[test]
fn state_storage_failure_rolls_back_the_pixel_candidate_and_allows_retry() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "atomic");
    let view = show(&mut w, &[base]);
    let count_before = count(&mut w);
    let db = rusqlite::Connection::open(dir.path().join(".retro-art/art.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_view BEFORE INSERT ON presentation_states WHEN json_extract(NEW.payload,'$.action')='paint' BEGIN SELECT RAISE(FAIL,'test write failure'); END;").unwrap();
    let input = paint(&view, 2, 2);
    assert_eq!(
        w.human_action(serde_json::from_value(input.clone()).unwrap())
            .err()
            .unwrap()
            .code,
        "storage_error"
    );
    assert_eq!(count(&mut w), count_before);
    assert_eq!(current(&mut w)["state_id"], view["state_id"]);
    db.execute_batch("DROP TRIGGER reject_view").unwrap();
    assert_ne!(act(&mut w, input)["state_id"], view["state_id"]);
    assert_eq!(count(&mut w), count_before + 1);
}
#[test]
fn multiple_pictures_save_together_and_readonly_items_cannot_be_painted() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let a = art(&mut w, "a");
    let b = art(&mut w, "b");
    let mut args = arguments(&mut w, &[a, b]);
    args["items"][1]["editable"] = json!(false);
    let view = call(&mut w, "present_art", args)["presentation"].clone();
    let mut bad = paint(&view, 1, 1);
    bad["item_index"] = json!(1);
    assert_eq!(
        w.human_action(serde_json::from_value(bad).unwrap())
            .err()
            .unwrap()
            .code,
        "invalid_input"
    );
    let edited = act(&mut w, paint(&view, 1, 1));
    let saved = act(&mut w, command(&edited, "save"));
    assert_eq!(saved["saved"]["art_ids"], edited["state"]["art_ids"]);
    assert_eq!(saved["state"]["art_ids"][1], view["state"]["art_ids"][1]);
    let mut too_big = arguments(
        &mut w,
        &[saved["state"]["art_ids"][0].as_str().unwrap().into()],
    );
    too_big["items"][0]["scale"] = json!(65);
    assert_eq!(
        w.call("present_art", too_big).err().unwrap().code,
        "invalid_input"
    );
}
#[test]
fn simultaneous_human_edits_accept_one_state_and_leave_the_other_a_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "shared");
    let view = show(&mut w, &[base]);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = (0..2)
        .map(|i| {
            let barrier = barrier.clone();
            let path = dir.path().to_owned();
            let input = paint(&view, i, 1);
            std::thread::spawn(move || {
                let mut w = Workspace::open(path).unwrap();
                barrier.wait();
                w.human_action(serde_json::from_value(input).unwrap())
                    .map(|_| ())
                    .map_err(|e| e.code)
            })
        })
        .collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert!(results.iter().any(|r| {
        r.as_ref()
            .err()
            .is_some_and(|e| e == "presentation_conflict")
    }));
    assert_eq!(count(&mut w), 2);
}

#[test]
fn playback_requires_explicit_timing_and_respects_the_combined_display_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let id = art(&mut w, "walk");
    let mut args = arguments(&mut w, std::slice::from_ref(&id));
    args["items"][0]["editable"] = json!(false);
    args["items"][0]["playback"] = json!([{"art_id":id,"label":"Idle"}]);
    assert_eq!(
        w.call("present_art", args.clone()).err().unwrap().code,
        "invalid_input"
    );
    args["items"][0]["playback"][0]["duration_ms"] = json!(120);
    assert!(
        call(&mut w, "present_art", args.clone())["presentation"]["is_current"]
            .as_bool()
            .unwrap()
    );
    let current = current(&mut w);
    args["expected_presentation_id"] = current["presentation_id"].clone();
    args["expected_state_id"] = current["state_id"].clone();
    args["items"][0]["scale"] = json!(64);
    args["items"][0]["playback"] = json!(vec![
        json!({"art_id":id,"label":"Idle","duration_ms":120});
        4
    ]);
    let other = args["items"][0].clone();
    args["items"].as_array_mut().unwrap().push(other);
    assert_eq!(
        w.call("present_art", args).err().unwrap().code,
        "limit_exceeded"
    );
}

fn mark(view: &Value, item_index: usize, pixels: Value, marked: bool) -> Value {
    let mut input = command(view, "mark");
    input["item_index"] = json!(item_index);
    input["pixels"] = pixels;
    input["marked"] = json!(marked);
    input
}

#[test]
fn concern_strokes_preserve_art_and_share_undo_save_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "concerns");
    let original = w.load(&base).unwrap();
    let view = show(&mut w, std::slice::from_ref(&base));
    let first = act(
        &mut w,
        mark(
            &view,
            0,
            json!([{"x":3,"y":2},{"x":1,"y":2},{"x":1,"y":2}]),
            true,
        ),
    );
    let expected = json!([{"item_index":0,"art_id":base,"bounds":{"x":1,"y":2,"width":3,"height":1},"pixels":[{"x":1,"y":2},{"x":3,"y":2}]}]);
    assert_eq!(first["state"]["concerns"], expected);
    assert_eq!(first["state"]["art_ids"], view["state"]["art_ids"]);
    assert_eq!(w.load(&base).unwrap().id().unwrap(), original.id().unwrap());
    assert_eq!(count(&mut w), 1);
    let same = act(&mut w, mark(&first, 0, json!([{"x":1,"y":2}]), true));
    assert_eq!(same["state_id"], first["state_id"]);
    let removed = act(&mut w, mark(&same, 0, json!([{"x":1,"y":2}]), false));
    assert_eq!(
        removed["state"]["concerns"][0]["pixels"],
        json!([{"x":3,"y":2}])
    );
    let undone = act(&mut w, command(&removed, "undo"));
    assert_eq!(undone["state"]["concerns"], expected);
    assert_eq!(undone["undo_available"], false);
    let saved = act(&mut w, command(&undone, "save"));
    assert_eq!(saved["saved"]["concerns"], expected);
    assert_eq!(saved["dirty"], false);
    drop(w);
    let mut w = Workspace::open(dir.path()).unwrap();
    assert_eq!(current(&mut w)["saved"], saved["saved"]);
    let painted = act(&mut w, paint(&saved, 1, 2));
    assert_eq!(
        painted["state"]["concerns"][0]["art_id"],
        painted["state"]["art_ids"][0]
    );
    assert_eq!(
        painted["state"]["concerns"][0]["pixels"],
        expected[0]["pixels"]
    );
    assert_eq!(painted["saved"]["concerns"], expected);
    let restored = act(&mut w, command(&painted, "undo"));
    assert_eq!(restored["state"]["art_ids"], view["state"]["art_ids"]);
    assert_eq!(restored["state"]["concerns"], expected);
}

#[test]
fn concern_marks_allow_readonly_art_but_reject_invalid_regions_and_total_overflow() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "bounds");
    let mut args = arguments(&mut w, std::slice::from_ref(&base));
    args["items"][0]["region"] = json!({"x":2,"y":3,"width":3,"height":2});
    args["items"][0]["editable"] = json!(false);
    let view = call(&mut w, "present_art", args)["presentation"].clone();
    let before = count(&mut w);
    let marked = act(&mut w, mark(&view, 0, json!([{"x":2,"y":3}]), true));
    for (input, code) in [
        (
            mark(&marked, 0, json!([{"x":3,"y":3},{"x":1,"y":3}]), true),
            "out_of_bounds",
        ),
        (mark(&marked, 0, json!([]), true), "limit_exceeded"),
        (
            mark(&marked, 0, json!(vec![json!({"x":2,"y":3}); 4097]), true),
            "limit_exceeded",
        ),
        (
            mark(&marked, 1, json!([{"x":2,"y":3}]), true),
            "invalid_input",
        ),
    ] {
        assert_eq!(
            w.human_action(serde_json::from_value(input).unwrap())
                .err()
                .unwrap()
                .code,
            code
        );
        assert_eq!(current(&mut w)["state_id"], marked["state_id"]);
    }
    assert_eq!(count(&mut w), before);
    let mut target = serde_json::to_value(w.load(&base).unwrap().target).unwrap();
    target["width"] = json!(64);
    target["height"] = json!(64);
    let large = call(
        &mut w,
        "create_art",
        json!({"target":target,"initial":{"kind":"fill","index":0}}),
    )["art_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut args = arguments(&mut w, &[large, base]);
    args["items"][0]["region"] = json!({"x":0,"y":0,"width":64,"height":64});
    args["items"][0]["scale"] = json!(1);
    let large_view = call(&mut w, "present_art", args)["presentation"].clone();
    let pixels: Vec<_> = (0..64)
        .flat_map(|y| (0..64).map(move |x| json!({"x":x,"y":y})))
        .collect();
    let full = act(&mut w, mark(&large_view, 0, json!(pixels), true));
    assert_eq!(
        full["state"]["concerns"][0]["pixels"]
            .as_array()
            .unwrap()
            .len(),
        4096
    );
    assert_eq!(
        w.human_action(
            serde_json::from_value(mark(&full, 1, json!([{"x":0,"y":0}]), true)).unwrap()
        )
        .err()
        .unwrap()
        .code,
        "limit_exceeded"
    );
    assert_eq!(current(&mut w)["state_id"], full["state_id"]);
    let unmarked = act(&mut w, mark(&full, 0, json!([{"x":0,"y":0}]), false));
    let second = act(&mut w, mark(&unmarked, 1, json!([{"x":0,"y":0}]), true));
    assert_eq!(second["state"]["concerns"].as_array().unwrap().len(), 2);
}

#[test]
fn concern_state_failures_retries_and_new_presentations_preserve_the_original_marks() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "atomic-marks");
    let view = show(&mut w, std::slice::from_ref(&base));
    let db = rusqlite::Connection::open(dir.path().join(".retro-art/art.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_mark BEFORE INSERT ON presentation_states WHEN json_extract(NEW.payload,'$.action')='mark' BEGIN SELECT RAISE(FAIL,'injected failure'); END;").unwrap();
    let input = mark(&view, 0, json!([{"x":1,"y":1}]), true);
    assert_eq!(
        w.human_action(serde_json::from_value(input.clone()).unwrap())
            .err()
            .unwrap()
            .code,
        "storage_error"
    );
    assert_eq!(current(&mut w)["state_id"], view["state_id"]);
    db.execute_batch("DROP TRIGGER reject_mark").unwrap();
    let marked = act(&mut w, input.clone());
    let saved = act(&mut w, command(&marked, "save"));
    let next = show(&mut w, std::slice::from_ref(&base));
    assert!(next["state"].get("concerns").is_none());
    assert_eq!(act(&mut w, input)["state_id"], next["state_id"]);
    assert_eq!(
        w.human_action(
            serde_json::from_value(mark(&marked, 0, json!([{"x":2,"y":1}]), true)).unwrap()
        )
        .err()
        .unwrap()
        .code,
        "presentation_conflict"
    );
    let archived = call(
        &mut w,
        "inspect_presentation",
        json!({"presentation_id":view["presentation_id"]}),
    )["presentation"]
        .clone();
    assert_eq!(archived["saved"], saved["saved"]);
    assert_eq!(count(&mut w), 1);
}

#[test]
fn legacy_presentation_payloads_keep_their_ids_and_undo_after_marking_is_added() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = Workspace::open(dir.path()).unwrap();
    let base = art(&mut w, "legacy-view");
    let view = show(&mut w, std::slice::from_ref(&base));
    let painted = act(&mut w, paint(&view, 1, 1));
    let mut legacy = painted["state"].clone();
    legacy.as_object_mut().unwrap().remove("concerns");
    legacy.as_object_mut().unwrap().remove("undo_concerns");
    // The original stored format hashes fields in this order, not a JSON map's key order.
    let payload = format!(
        r#"{{"presentation_id":{},"art_ids":{},"previous_state_id":{},"undo_art_ids":{},"action":{}}}"#,
        legacy["presentation_id"], legacy["art_ids"], legacy["previous_state_id"],
        legacy["undo_art_ids"], legacy["action"]
    ).into_bytes();
    let legacy_id = format!("view_state_{}", dotmend::art::digest(&payload));
    let db = rusqlite::Connection::open(dir.path().join(".retro-art/art.sqlite")).unwrap();
    db.execute(
        "INSERT OR IGNORE INTO presentation_states(id,payload) VALUES (?1,?2)",
        rusqlite::params![legacy_id, payload],
    )
    .unwrap();
    db.execute(
        "UPDATE presentations SET head_id=?1,saved_state_id=?1 WHERE id=?2",
        rusqlite::params![legacy_id, view["presentation_id"].as_str().unwrap()],
    )
    .unwrap();
    drop(w);
    let mut w = Workspace::open(dir.path()).unwrap();
    let restored = current(&mut w);
    assert_eq!(restored["state_id"], legacy_id);
    assert_eq!(restored["saved"]["state_id"], legacy_id);
    let undone = act(&mut w, command(&restored, "undo"));
    assert_eq!(undone["state"]["art_ids"][0], base);
    let stored: Vec<u8> = db
        .query_row(
            "SELECT payload FROM presentation_states WHERE id=?1",
            [&legacy_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, payload);
    let marked = act(&mut w, mark(&undone, 0, json!([{"x":1,"y":1}]), true));
    assert_eq!(marked["state"]["concerns"][0]["art_id"], base);
}
