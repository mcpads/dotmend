# Editing art through MCP

This guide is served verbatim at `dotmend://guides/editing`. Use `tools/list` for exact input and output schemas. MCP uses `2026-07-28`: send protocolVersion and clientCapabilities in each request's reserved `_meta` fields; `server/discover` is optional and `initialize` is not supported. Server-authored instructions, descriptions and errors are English. Preserve caller-authored text in its original language, including non-English instructions, labels and feedback.

Art coordinates start at the top-left `(0,0)`, with `+x` right and `+y` down. The right and bottom edges of `{x,y,width,height}` are exclusive. Reference images have their own source coordinates.

## Project workspace

Dotmend uses the project directory supplied by the client at launch, or an explicit `--workspace`. Art, sources, exports and screen records live in that project's `.dotmend/`. Sessions in the same project share these records; different projects remain independent. Input file paths are relative to the project directory. New storage creates its own `.gitignore` to keep work out of Git; existing ignore rules are preserved.

## Presenting work to a human

Humans paint with palette colors, mark suspected issues, undo the last stroke once and save. They can also choose several alternatives when you offer candidate choices. Handle collections, filters, past candidates, regions and reviews through conversation and tools.

When first sharing the screen, briefly explain its available controls in the user's language: choose a palette color, then click or drag to paint; **Mark issues** uses left clicks/drags to add marks and right clicks/drags to remove them; **Undo** (Ctrl/Cmd+Z) reverses the last stroke once; **Save** (Ctrl/Cmd+S) records the current result. A click or drag is one stroke, and painting and marking share the same undo. Tell the user to ask you for older results or other operations. Adapt the explanation to read-only and playback views; repeat it only if asked or needed. Do not send the user to this guide or require IDs and technical settings.

For more than 20 allowed colors, the screen starts in **Mark issues** mode and shows up to 20 commonly used palette entries, including Erase when allowed. Explain this briefly in the user's language and invite them to mark areas and describe the desired change. Read saved concerns, focus the marked regions, resolve unclear intent, apply edits within the request's permissions and present the result. Keep color selection and palette details with the agent. The displayed subset stays fixed during editing; all original indices remain available to agent tools.

1. Choose a fresh random `control_id` per independent task (16..128 ASCII letters, digits, underscores or hyphens); retain it across calls and connections. Call `open_workbench({control_id,work_state:"working"})` for the instance ID and URL. Do not bypass another owner or the shared limit. Read `inspect_presentation({})`; no existing view returns `presentation:null`.
2. Prepare `view.title`, `note` and `items` to describe the user's task. An art item is `{kind:"art",art_id,label,region,scale,editable,request_id?}`. Attach `request_id` to an editable item carrying out a protected request. Copy the freshly read IDs into `view.expected_presentation_id` and `view.expected_state_id`; both keys must be explicit `null` for the first view.
3. Call `present_art({control_id,workbench_id,view})`. On conflict, read the human's latest edits first. When ready for their input, call `open_workbench({control_id,work_state:"waiting"})` and share its returned URL.
4. Read the explicit save from `inspect_presentation` at `saved.art_ids` and `saved.state_id`. Current drafts are at `state.art_ids`; `dirty` distinguishes them from the last explicit save. Saving is not acceptance.

Do not close the screen while the human is working. Use `open_workbench({control_id,work_state:"working"})` at each follow-up before doing art work or calling an external generator. A declared working task stays alive through long calls and gaps between calls, including external work. Collaborators must use the same explicit task ID to change its work state; ordinary art calls never imply task ownership or activity.

Before handing the screen to the human or awaiting their reply, call `open_workbench({control_id,work_state:"waiting"})`. Share only the URL returned by that call, never a URL from memory or an earlier answer. On failure, cancellation or interruption, switch to waiting or close the finished task. The server cannot infer that an external agent stopped: leaving working set will hold the screen while the host lives. Use `inspect_workbench({control_id})` to read the current instance and work_state without renewing activity. Finish with `close_workbench({control_id,workbench_id})` and check its result.

Reopening preserves an active instance's ID, URL and timeout, and resets idle time. Omitting work_state preserves an active state; a new instance defaults to waiting. In waiting, presentation, successful human actions and real visible input (scrolling, color selection and unfinished dragging) renew activity. Art calls, failed calls, HTTP/browser polling, playback and inspect_workbench do not. The waiting idle timeout defaults to 1800 seconds, configurable from 1 to 1800. The hosting MCP process exiting always ends HTTP, including working tasks; art and drafts remain. After shutdown, open a new instance and inspect the preserved draft before continuing.

One explicit controller owns each workspace's screen, with a shared maximum of four. The same control_id works across MCP connections; different tasks remain separate even on one connection. Share the workbench URL with the human; keep control IDs private. Independent candidate creation does not require screen control. Never bypass control using shell servers, alternate runtime directories or lock-file changes. The local HTTP interface is not an MCP transport or a public agent mutation API. Closing a forwarding MCP connection does not close the host's screen. If close returns closing, inspect until closed.

For collections and filters, use `list_art` and present the selected candidates in order. For history, inspect parent IDs or archived presentations and states. `inspect_presentation({presentation_id,state_id})` is read-only; use `present_art` to change the visible screen. A response with `is_current:false` is not the current screen.

Reference items use `{kind:"reference",art_id,source_hash,label,region,scale}` and source-image coordinates; they are read-only. For playback, supply existing frames in a read-only art item's `playback`, show the full first `art_id`, and specify every frame's `duration_ms`. If timing is unknown, show frames side by side. All playback frames count toward the total display area. Prepare narrower crops or native-size views yourself; users should not have to learn IDs, coordinates or validation settings.

## Offering candidate choices

Create alternatives as separate candidates, keeping the supplied intent and constraints. Label the differences clearly and show comparable regions at appropriate scales. The agent prepares the alternatives; choosing does not generate images automatically.

Use the existing `present_art` view with `candidate_choices: {"item_indices":[1,2,3]}` to make those zero-based items selectable. For example, item 0 can be the original and items 1..3 the alternatives. Offer 2..16 distinct static art candidates with `editable:false`. References, playback and editable items cannot be choices. Other items can still provide context. Omit `candidate_choices` for an ordinary editing view.

Tell the human in their language to click **Choose** under any candidates they want to explore, then **Save** (Ctrl/Cmd+S). They can choose several and click a chosen option again to remove it. Nothing is preselected. Choice changes are preserved immediately as drafts; Save communicates the current choice. Mark issues remains available for details. Undo affects only the last painting or marking stroke and leaves candidate choices unchanged.

Read `saved.chosen_candidates`, a list of `{item_index,art_id}`, together with `saved.state_id` and `presentation_id`. `state.chosen_candidates` may contain a newer draft; absent or empty fields mean no choice. Do not infer choices from `saved.art_ids`, which includes every shown item. An empty choice is not rejection of the alternatives. Read the saved marks and any conversational feedback as well.

Use the chosen IDs as the exact bases for separate refinements, preserving their target constraints and applicable permissions. Recreate any pixel selections and protection against each new base before protected edits. Show the next alternatives through a new presentation, which starts with no chosen candidates. Previous candidates, saved choices and draft states remain available through `inspect_presentation`. Do not discard unchosen alternatives or treat them as rejected.

Choices express directions for further work. They do not submit results, record final acceptance, grant additional edit permission, validate constraints or approve game insertion. The agent continues the conversation using the saved choice; Dotmend does not invoke the agent automatically. On conflicts or lost responses, inspect the current presentation before retrying and never apply an old click to a different view.

## Reading human issue marks

The human toggles **Mark issues** to add pixels with a left click or drag and remove them with a right click or drag. Choosing a palette color returns to painting. Marks are a separate overlay, not palette edits. Static art can be marked even when read-only or protected. Reference images and playback cannot be marked; present a specific still-art candidate when needed.

Read `state.concerns` for current draft marks or `saved.concerns` with `saved.state_id` for the explicit save. Missing concern fields mean an empty list. Each entry is `{item_index,art_id,bounds,pixels:[{x,y}]}` in full-art coordinates. `bounds` encloses the exact marked pixels for observation; it does not mean every pixel in the rectangle was marked. Pass the entry's `art_id` and `bounds` to `focus_art` with explicit `context_padding` and `scale`. Use its exact `pixels` if a later, authorized request needs a selection.

Within one presentation, human painting retains marked coordinates and associates them with that state's new candidate. It does not automatically resolve an issue. Read the prior state or explicit save to recover the previous candidate and marks. New agent presentations start without marks; old presentation states remain available. Do not silently transfer marks to a different candidate or frame.

Marking is atomic per stroke, shares one undo with painting, and persists before Save. Undo restores both art and marks from the preceding changing stroke. Duplicate additions and removal of unmarked pixels do not consume undo. The limit is 4096 input pixels per marking stroke and 4096 marked pixels across a presentation.

Marks request observation. They are not a failed constraint check, permission to edit, a protection change, rejection, or acceptance. Inspect the marked candidate and follow the existing request and the user's expressed intent before editing or recording a judgment.

## Starting from a request

1. Find requests with `list_edit_requests({status:"pending"})`. Follow `next_cursor` with the same filter.
2. Read `inspect_edit_request({request_id})` for the base, intent, target, selections, protection, references and feedback. `previous_review` is the pinned review that led to this request; `latest_review` is the current judgment of this request's result. Distinguish follow-up instructions from earlier requests.
3. Check target dimensions, palette **order**, transparent index, allowed indices and additional constraints. Equal RGB values do not make two indices interchangeable. Do not remove unknown constraints or invent mandatory palette roles.
4. Call `validate_art({art_id})`. `fail` means an observed violation; `unknown` means an unsupported constraint. Both block export. Malformed check parameters produce a tool input error.

## Focusing and selecting

Call `focus_art` with the actual candidate ID, `region`, `context_padding` and `scale`. Add `include_indices`, `grid`, `compare_to_art_id` or `reference` when needed. Diagnostic `locations[].region` identifies an observation area, not write permission. A baseline location with role `expected` is the required row, not necessarily a violating pixel.

`views[].image_index` counts image blocks only. Convert display positions through the returned region and scale to full-art coordinates before editing. A reference view with `source_pixel_top_left_xy` has a `source_hash` rather than a target `art_id`. Specify source and target regions independently.

Use `create_selection` to store a rectangle, explicit pixels or a connected region. Connected selections require a seed, palette indices and 4- or 8-neighbor connectivity. Read the exact mask from the returned `mask_uri` through `resources/read`. A selection is pinned to its base candidate; create a new one for a different base or frame.

Assign roles through `request_edit.write_selection_id` and `protected_selection_ids`. Link a new instruction with `previous_request_id`, and a particular feedback record with `previous_review_id`. Existing requests and protection remain immutable.

## Editing with small actions

### Choose the repair before changing pixels

First inspect the whole image at `scale:1`, then focus on the issue with context. State one visual problem, the intended improvement, and the features to preserve. Treat an aesthetic diagnosis as a hypothesis, distinct from a failed constraint check. Use supplied references and palette roles; do not infer missing game rules or character details.

| Observed problem | Repair to try |
| --- | --- |
| A wrong color or isolated pixel, with the shape still correct | Change only the relevant indices or pixels. |
| A broken contour, proportion, gap, or cluster of pixels | Reconstruct the smallest meaningful region that contains the shape and its boundary. |
| A misplaced fragment whose shape is already useful | Copy from a fixed candidate and restore its old location in one action. |
| Uncertain interpretation or several plausible shapes | Make two alternatives from the same baseline and compare them before continuing. |

Reconstruction does not require a separate erase call. Prefer a complete `paint_rows` replacement when the final patch is known, with `null` only where pixels must stay untouched. Explicitly replace obsolete pixels as well as adding new ones. If clearing and drawing are separate operations, put both in one `edit_art` call so a failure leaves the baseline intact and a blank intermediate is not presented to the human.

Clear only to an explicitly allowed transparent index or a known background/restored pixel value. Index 0 is not automatically transparent; targets without transparency still need valid palette indices. For irregular writable regions, use explicit pixels or rows that skip protection. A broad `fill_rect` crossing protected pixels fails even if later operations restore them.

Keep the baseline ID, bind the request's selection and protection, and change one visual hypothesis per candidate. For a structural repair, work from silhouette and major color regions toward contour and small accents, inspecting each completed pass. Preserve the chosen lighting, outline, and distinguishing features supplied by the task. Do not keep adding highlights or isolated pixels merely because a previous attempt looked wrong.

Compare against the baseline after each action, first at actual size and then enlarged; include related frames when relevant. Check the intended improvement, boundary continuity, preserved details, and unintended visible changes separately from numeric constraint validation. A smaller diff or a passing validator does not prove better art. If the repair does not help, branch again from the retained baseline instead of accumulating corrections on it. Ask for human judgment through conversation when the alternatives remain ambiguous; saving alone is not a judgment.

### Apply and inspect an action

Pass `request_id`, the current `art_id`, a permitted `write_region` and `operations` to `edit_art`. Omitting `request_id` creates an independent edit without that request's protection. Always include it when carrying out a protected request.

- `set_pixels` changes exact coordinates and indices.
- `paint_rows` writes a rectangular array; `null` preserves a pixel. Transparency uses the integer transparent index.
- `replace_index` replaces matching indices within a region.
- `paste` copies an immutable source region. `over` skips source transparency; `replace` writes every pixel. Palette and transparent index must match.
- `fill_rect` writes the whole rectangle. Overlapping protection fails the entire action.

Any attempted write outside the permitted region or into protection fails the complete action, even when writing the same index or restoring it in a later operation. Keep the returned candidate ID and use it for the next action. A no-op returns the existing ID. Resume from an earlier candidate ID to undo agent edits.

Check changed indices and visible differences with `compare_art` or comparison in `focus_art`. After a local edit, inspect the whole art using `render_art` at `scale:1`. Geometric validity and aesthetic judgment are separate.

## Applying edits across assets

`render_art_set` compares explicit candidates, order and optional anchors. Supply durations only when known. Visual comparability does not establish edit compatibility.

Use `edit_art_set` with `preview:true` before applying shared coordinates and operations. For protected work, provide exactly one `request_bindings` entry per candidate. Apply the same inputs with the returned `plan_hash` as `expected_plan_hash`. Any incompatible target or protection violation leaves the whole set unchanged. Do not assume corresponding parts of different poses use the same coordinates.

## Importing, submitting and recovering

For externally generated images, use `prepare_image` with explicit crop, resize, palette mapping and transparency settings. It accepts local RGB/RGBA PNG input; generation success does not establish target validity. `attach_reference` preserves the original or reference without changing art pixels. `create_art` can create exact index data or import an exported bundle.

Submit the final candidate with `submit_edit_result({request_id,result_art_id,notes})`. Submission checks ancestry and preservation outside the permitted pixels; it does not record acceptance. Use `review_edit_result` only for the user's expressed judgment, notes and regional feedback, with the freshly read `expected_review_id`. Read reviews and follow-ups through `inspect_edit_request`.

After a lost response, inspect ownership, requests, candidates and provenance before retrying. An observation failure does not authorize replaying an edit on a new base. Identical submission request, candidate and notes reuse the same result; different data conflicts. For `plan_mismatch`, preview the same inputs again. For protection errors, inspect the operation, coordinates and request selections. A changed intent requires a new request.

`export_art` requires every mandatory check to pass. Distinguish preview PNG from exact palette-index data. Report only observed stages: candidate creation, constraint validation, human acceptance and actual game verification.

## Limits and errors

Canvas dimensions are at most 1024 per side and 262144 total pixels. An edit call has at most 256 operations and a total write budget. Sets and frame lists have at most 16 items. Inspect `list_art.limits` and the tool schemas for full limits.

Preview scale is 1..64. Combined focus or frame output is at most 1048576 pixels, including references. Reduce regions, padding, scale or optional outputs explicitly when needed. Index lookup and explicit selection input are limited to 4096 pixels each. Follow history cursors; partial observation is not a complete review.

Each MCP process admits at most 32 concurrent tool operations, with a burst budget of 64 replenished at 64 calls per second. A rejected call returns a tool error with code `rate_limited` and `details.retry_after_ms`; await pending work and honor that delay. Dependent edits must remain sequential.

Tool execution failures return `isError:true` and matching JSON in `structuredContent` and text content. Protocol errors use JSON-RPC errors. Read `error.code` and structured details to recover; do not parse English message wording as a stable identifier. Caller-authored text and diagnostic values retain their original contents and language.


### Import a PNG

1. Inspect an existing target art, or call `create_art` with the supplied target and `initial: {"kind":"fill","index":<allowed index>}`. Palette colors use `#RRGGBB`; `transparent_index` carries transparency. Obtain missing required target conditions from the caller.
2. Call `prepare_image` with that `target_art_id`, a workspace-relative `source_path`, and explicit crop, resize, alpha, color mapping and dither settings.
3. Use the returned `art_id` for inspection, validation, editing and presentation. Read conversion diagnostics and verify game index identity when required.

On a path error, use `details.workspace_root` to copy the file to a non-conflicting destination, verify its bytes and retry the relative path. Preserve the source. Report unavailable file access if copying cannot proceed. Keep existing images in the file workflow; reserve inline indices for caller-supplied index data and `bundle_path` for exported Dotmend art.
