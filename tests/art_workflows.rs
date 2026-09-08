#![cfg(feature = "native")]

use dotmend::{pixels::*, workspace::Workspace};
use serde_json::{Value, json};
use std::fs;

fn target(name: &str) -> Value {
    json!({"resource_id":name,"width":4,"height":3,"palette":["#000000","#000000","#FF0000","#FF0000","#FFFFFF"],"transparent_index":0,"allowed_indices":[0,1,2,3],"constraints_ref":null,"requirements":[]})
}
fn create(workspace: &mut Workspace, name: &str) -> String {
    workspace.call("create_art",json!({"target":target(name),"initial":{"kind":"fill","index":0},"context":{"group_id":"characters","variant":name}})).unwrap().data["art_id"].as_str().unwrap().to_owned()
}
fn edit(workspace: &mut Workspace, id: &str, x: u32, y: u32, index: u16) -> String {
    workspace.call("edit_art",json!({"art_id":id,"write_region":{"x":x,"y":y,"width":1,"height":1},"operations":[{"kind":"set_pixels","pixels":[{"x":x,"y":y,"index":index}]}]})).unwrap().data["art_id"].as_str().unwrap().to_owned()
}
fn listed_ids(workspace: &mut Workspace) -> Vec<String> {
    workspace
        .call("list_art", json!({"limit":100}))
        .unwrap()
        .data["arts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["art_id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn editing_is_atomic_and_intermediate_candidates_survive_restart_and_branching() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let original = create(&mut workspace, "hero.front");
    let first = edit(&mut workspace, &original, 1, 1, 2);
    let second = edit(&mut workspace, &first, 2, 1, 3);
    let before = listed_ids(&mut workspace);
    let error = workspace.call("edit_art",json!({"art_id":second,"write_region":{"x":0,"y":0,"width":4,"height":3},"operations":[{"kind":"fill_rect","rect":{"x":0,"y":0,"width":1,"height":1},"index":2},{"kind":"set_pixels","pixels":[{"x":4,"y":0,"index":1}]}]})).err().unwrap();
    assert_eq!(error.code, "out_of_bounds");
    assert_eq!(listed_ids(&mut workspace), before);
    assert_eq!(
        workspace.load(&original).unwrap().indices,
        vec![vec![0; 4]; 3]
    );
    drop(workspace);
    let mut workspace = Workspace::open(temp.path()).unwrap();
    assert_eq!(
        workspace.load(&second).unwrap().parents,
        vec![first.clone()]
    );
    assert_eq!(workspace.load(&first).unwrap().indices[1], vec![0, 2, 0, 0]);
    let branch = edit(&mut workspace, &first, 0, 0, 1);
    assert_eq!(
        workspace.load(&branch).unwrap().indices[1],
        vec![0, 2, 0, 0]
    );
    assert_eq!(
        workspace.load(&second).unwrap().indices[1],
        vec![0, 2, 3, 0]
    );
}

#[test]
fn palette_identity_and_export_roundtrip_preserve_duplicate_colors_and_transparency() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let original = create(&mut workspace, "icon");
    let first = edit(&mut workspace, &original, 1, 1, 2);
    let second = edit(&mut workspace, &first, 1, 1, 3);
    let comparison = workspace
        .call(
            "compare_art",
            json!({"before_art_id":first,"after_art_id":second}),
        )
        .unwrap();
    assert_eq!(comparison.data["changed_pixels"], 1);
    assert_eq!(comparison.data["visual_changed_pixels"], 0);
    let opaque = edit(&mut workspace, &second, 0, 0, 1);
    let raster = art_raster(&workspace.load(&opaque).unwrap()).unwrap();
    assert_eq!(&raster.rgba[0..4], &[0, 0, 0, 255]);
    assert_eq!(&raster.rgba[4..8], &[0, 0, 0, 0]);
    let output = workspace
        .call("export_art", json!({"art_id":opaque}))
        .unwrap();
    let path = output.data["bundle_path"].as_str().unwrap();
    let other = tempfile::tempdir().unwrap();
    fs::create_dir(other.path().join("bundle")).unwrap();
    for entry in fs::read_dir(temp.path().join(path)).unwrap() {
        let entry = entry.unwrap();
        fs::copy(
            entry.path(),
            other.path().join("bundle").join(entry.file_name()),
        )
        .unwrap();
    }
    let mut imported = Workspace::open(other.path()).unwrap();
    let result = imported
        .call("create_art", json!({"bundle_path":"bundle"}))
        .unwrap();
    let result = imported
        .load(result.data["art_id"].as_str().unwrap())
        .unwrap();
    let expected = workspace.load(&opaque).unwrap();
    assert_eq!(result.target, expected.target);
    assert_eq!(result.indices, expected.indices);
    fs::write(other.path().join("bundle/art.json"), b"{}").unwrap();
    assert_eq!(
        imported
            .call("create_art", json!({"bundle_path":"bundle"}))
            .err()
            .unwrap()
            .code,
        "integrity_error"
    );
}

#[test]
fn invalid_and_unknown_constraints_are_never_reported_as_exportable() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let mut condition = target("unknown");
    condition["requirements"] = json!([{"id":"tile_budget","kind":"tile_count","parameters":{"maximum":2},"source_ref":null}]);
    let created = workspace
        .call(
            "create_art",
            json!({"target":condition,"initial":{"kind":"fill","index":0}}),
        )
        .unwrap();
    let id = created.data["art_id"].as_str().unwrap();
    assert_eq!(
        workspace
            .call("validate_art", json!({"art_id":id}))
            .unwrap()
            .data["status"],
        "unknown"
    );
    assert_eq!(
        workspace
            .call("export_art", json!({"art_id":id}))
            .err()
            .unwrap()
            .code,
        "validation_blocked"
    );
    let mut missing = target("missing");
    missing.as_object_mut().unwrap().remove("transparent_index");
    assert!(
        workspace
            .call(
                "create_art",
                json!({"target":missing,"initial":{"kind":"fill","index":0}})
            )
            .is_err()
    );
    assert_eq!(
        workspace
            .call(
                "create_art",
                json!({"target":target("reserved"),"initial":{"kind":"fill","index":4}})
            )
            .err()
            .unwrap()
            .code,
        "invalid_index"
    );
}

#[test]
fn image_conversion_is_reproducible_and_keeps_opaque_pixels_out_of_the_transparent_slot() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let original = create(&mut workspace, "portrait");
    let source = Raster {
        width: 2,
        height: 1,
        rgba: vec![0, 0, 0, 255, 255, 0, 0, 0],
    };
    let bytes = encode_png(&source).unwrap();
    fs::write(temp.path().join("input.png"), &bytes).unwrap();
    let args = json!({"source_path":"input.png","target_art_id":original,"transform":{"crop":{"x":0,"y":0,"width":2,"height":1},"resize":"nearest","alpha":{"mode":"threshold","cutoff":128},"color_mapping":{"method":"nearest_rgb","opaque_indices":[3,2,1]},"dither":"none"}});
    let a = workspace.call("prepare_image", args.clone()).unwrap();
    let b = workspace.call("prepare_image", args).unwrap();
    assert_eq!(a.data["art_id"], b.data["art_id"]);
    let art = workspace.load(a.data["art_id"].as_str().unwrap()).unwrap();
    assert_eq!(art.indices, vec![vec![1, 1, 0, 0]; 3]);
    assert_eq!(fs::read(temp.path().join("input.png")).unwrap(), bytes);
    fs::write(temp.path().join("input.png"), b"changed").unwrap();
    drop(workspace);
    let workspace = Workspace::open(temp.path()).unwrap();
    assert_eq!(
        workspace.source(&art.references[0].source_hash).unwrap(),
        bytes
    );
    let after_render = workspace.load(a.data["art_id"].as_str().unwrap()).unwrap();
    render(
        &after_render,
        None,
        8,
        &Background::Solid {
            rgb: "#FFFFFF".into(),
        },
        true,
    )
    .unwrap();
    assert_eq!(
        workspace
            .load(a.data["art_id"].as_str().unwrap())
            .unwrap()
            .indices,
        art.indices
    );
}

#[test]
fn set_preview_is_non_mutating_and_requires_the_same_plan_for_atomic_apply() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let a = create(&mut workspace, "hero.front");
    let b = create(&mut workspace, "hero.back");
    let before = listed_ids(&mut workspace);
    let mut args = json!({"template_art_id":a,"art_ids":[a,b],"write_region":{"x":1,"y":1,"width":1,"height":1},"operations":[{"kind":"fill_rect","rect":{"x":1,"y":1,"width":1,"height":1},"index":2}],"preview":true});
    let preview = workspace.call("edit_art_set", args.clone()).unwrap();
    assert_eq!(preview.data["compatible"], true);
    assert_eq!(listed_ids(&mut workspace), before);
    args["preview"] = json!(false);
    args["expected_plan_hash"] = json!("different");
    assert_eq!(
        workspace
            .call("edit_art_set", args.clone())
            .err()
            .unwrap()
            .code,
        "plan_mismatch"
    );
    assert_eq!(listed_ids(&mut workspace), before);
    args["expected_plan_hash"] = preview.data["plan_hash"].clone();
    let applied = workspace.call("edit_art_set", args).unwrap();
    for item in applied.data["results"].as_array().unwrap() {
        let id = item["art_id"].as_str().unwrap();
        assert_eq!(workspace.load(id).unwrap().indices[1][1], 2);
        assert_eq!(
            workspace
                .load(item["base_art_id"].as_str().unwrap())
                .unwrap()
                .indices[1][1],
            0
        );
    }
    let mut mismatch = target("different_palette");
    mismatch["palette"][2] = json!("#0000FF");
    let other = workspace
        .call(
            "create_art",
            json!({"target":mismatch,"initial":{"kind":"fill","index":0}}),
        )
        .unwrap()
        .data["art_id"]
        .clone();
    let before = listed_ids(&mut workspace);
    let incompatible = json!({"template_art_id":a,"art_ids":[a,other],"write_region":{"x":0,"y":0,"width":1,"height":1},"operations":[{"kind":"set_pixels","pixels":[{"x":0,"y":0,"index":3}]}],"preview":false,"expected_plan_hash":"ignored"});
    assert_eq!(
        workspace
            .call("edit_art_set", incompatible)
            .err()
            .unwrap()
            .code,
        "validation_blocked"
    );
    assert_eq!(listed_ids(&mut workspace), before);
}

#[test]
fn human_edit_requests_pin_the_baseline_and_reject_changes_outside_the_selected_region() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let base = create(&mut workspace, "face");
    let request = workspace.call("request_edit",json!({"base_art_id":base,"write_region":{"x":1,"y":1,"width":1,"height":1},"instruction":"Brighten the eye"})).unwrap();
    let wrong = edit(&mut workspace, &base, 0, 0, 2);
    assert_eq!(
        workspace
            .call(
                "submit_edit_result",
                json!({"request_id":request.data["request_id"],"result_art_id":wrong,"notes":""})
            )
            .err()
            .unwrap()
            .code,
        "out_of_bounds"
    );
    let corrected = edit(&mut workspace, &base, 1, 1, 3);
    let submitted = workspace.call("submit_edit_result",json!({"request_id":request.data["request_id"],"result_art_id":corrected,"notes":"Updated the specified pixel"})).unwrap();
    assert_eq!(submitted.data["human_review"], "pending");
    drop(workspace);
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let requests = workspace
        .call("list_edit_requests", json!({"status":"submitted"}))
        .unwrap();
    assert_eq!(requests.data["requests"][0]["request"]["base_art_id"], base);
    assert_eq!(requests.data["requests"][0]["result_art_id"], corrected);
    assert_eq!(workspace.load(&base).unwrap().indices, vec![vec![0; 4]; 3]);
}

#[test]
fn paste_and_sparse_paint_preserve_unselected_pixels_and_palette_rules() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let base = create(&mut workspace, "sprite");
    let src = edit(&mut workspace, &base, 1, 0, 2);
    let destination = edit(&mut workspace, &base, 0, 1, 3);
    let result = workspace.call("edit_art",json!({"art_id":destination,"write_region":{"x":0,"y":1,"width":2,"height":1},"operations":[{"kind":"paste","source_art_id":src,"source_rect":{"x":0,"y":0,"width":2,"height":1},"x":0,"y":1,"flip_x":false,"flip_y":false,"mode":"over"},{"kind":"paint_rows","x":0,"y":1,"rows":[[null,1]]}]})).unwrap();
    assert_eq!(
        workspace
            .load(result.data["art_id"].as_str().unwrap())
            .unwrap()
            .indices[1],
        vec![3, 1, 0, 0]
    );
}

#[test]
fn paginated_art_queries_keep_their_snapshot_when_new_candidates_arrive() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let a = create(&mut workspace, "a");
    let b = create(&mut workspace, "b");
    let page = workspace.call("list_art", json!({"limit":1})).unwrap();
    assert_eq!(page.data["arts"][0]["art_id"], a);
    create(&mut workspace, "c");
    let next = workspace
        .call(
            "list_art",
            json!({"limit":1,"cursor":page.data["next_cursor"]}),
        )
        .unwrap();
    assert_eq!(next.data["arts"][0]["art_id"], b);
    assert!(next.data["next_cursor"].is_null());
}

#[test]
fn a_storage_failure_rolls_back_every_candidate_in_a_set() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let first = create(&mut workspace, "first");
    let second = create(&mut workspace, "second");
    let before = listed_ids(&mut workspace);
    let mut args = json!({"template_art_id":first,"art_ids":[first,second],"write_region":{"x":0,"y":0,"width":1,"height":1},"operations":[{"kind":"fill_rect","rect":{"x":0,"y":0,"width":1,"height":1},"index":2}],"preview":true});
    let preview = workspace.call("edit_art_set", args.clone()).unwrap();
    // Force failure after the first INSERT has succeeded inside the real action.
    let database = rusqlite::Connection::open(temp.path().join(".retro-art/art.sqlite")).unwrap();
    database.execute_batch("CREATE TRIGGER fail_second BEFORE INSERT ON arts WHEN NEW.resource_id='second' BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;").unwrap();
    args["preview"] = json!(false);
    args["expected_plan_hash"] = preview.data["plan_hash"].clone();
    assert_eq!(
        workspace
            .call("edit_art_set", args.clone())
            .err()
            .unwrap()
            .code,
        "storage_error"
    );
    drop(workspace);
    let mut workspace = Workspace::open(temp.path()).unwrap();
    assert_eq!(listed_ids(&mut workspace), before);
    assert_eq!(workspace.load(&first).unwrap().indices[0][0], 0);
    assert_eq!(workspace.load(&second).unwrap().indices[0][0], 0);
    database.execute_batch("DROP TRIGGER fail_second;").unwrap();
    assert!(workspace.call("edit_art_set", args).is_ok());
}

#[test]
fn simultaneous_clients_can_initialize_and_write_one_workspace() {
    use std::sync::{Arc, Barrier};
    // Reproduce independently starting MCP and UI processes against a fresh database.
    for _ in 0..8 {
        let temp = tempfile::tempdir().unwrap();
        let start = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|index| {
                    let start = start.clone();
                    let path = temp.path();
                    scope.spawn(move || {
                        start.wait();
                        let mut workspace = Workspace::open(path).unwrap();
                        create(&mut workspace, &format!("asset.{index}"))
                    })
                })
                .collect();
            let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            let mut workspace = Workspace::open(temp.path()).unwrap();
            let saved = listed_ids(&mut workspace);
            for id in ids {
                assert!(saved.contains(&id));
            }
        });
    }
}

#[test]
fn focusing_is_read_only_and_does_not_expand_an_edit_request() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::open(temp.path()).unwrap();
    let original = create(&mut workspace, "focus.hero");
    let region = json!({"x":1,"y":1,"width":1,"height":1});
    let requested = workspace
        .call(
            "request_edit",
            json!({"base_art_id":original,"write_region":region,"instruction":"Edit only this pixel"}),
        )
        .unwrap();
    let before = listed_ids(&mut workspace);
    let requests_before = workspace
        .call("list_edit_requests", json!({}))
        .unwrap()
        .data;
    for padding in [0, 3] {
        let focused = workspace.call("focus_art",json!({"art_id":original,"region":region,"context_padding":padding,"scale":4,"include_indices":true})).unwrap();
        assert_eq!(focused.data["indices"], json!([[0]]));
        assert_eq!(focused.data["focus_usage"], json!([1, 0, 0, 0, 0]));
    }
    assert_eq!(listed_ids(&mut workspace), before);
    assert_eq!(
        workspace
            .call("list_edit_requests", json!({}))
            .unwrap()
            .data,
        requests_before
    );
    let error = workspace.call("edit_art",json!({"art_id":original,"write_region":region,"operations":[{"kind":"set_pixels","pixels":[{"x":0,"y":0,"index":2}]}]})).err().unwrap();
    assert_eq!(error.code, "out_of_bounds");
    assert_eq!(listed_ids(&mut workspace), before);
    let edited = edit(&mut workspace, &original, 1, 1, 2);
    workspace.call("submit_edit_result",json!({"request_id":requested.data["request_id"],"result_art_id":edited,"notes":"Updated the specified pixel"})).unwrap();
    assert_eq!(
        workspace.load(&original).unwrap().indices,
        vec![vec![0; 4]; 3]
    );
}
