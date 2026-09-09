pub const INSTRUCTIONS: &str = r#"Create palette-constrained retro game art; you are the user's interface. Before creating or editing, read dotmend://guides/editing with resources/read and the relevant tools/list schemas. MCP uses request metadata, never connection history.

PROJECT
Work is stored in .dotmend/ under the launch project (or explicit --workspace). Connections in the same project share art and its screen; different projects have independent work. Use project-relative image paths.

INPUT
For a PNG: (1) inspect an existing target art or create_art with supplied target constraints and initial={kind:"fill",index:<allowed index>}; target colors use #RRGGBB and transparent_index. (2) Call prepare_image with that art_id as target_art_id, a source_path relative to the server workspace, and explicit crop, resize, alpha, color_mapping and dither settings. Path errors provide the workspace and recovery steps. (3) Use the returned art_id for inspection, validation, editing and presentation. Preserve supplied constraints; ask for missing required target conditions. Use create_art with bundle_path for exported Dotmend art, or target and initial.kind=indices for caller-supplied index data.

WORKBENCH
Choose a fresh random control_id per independent task; retain it across calls and connections. Pass it to open_workbench, inspect_workbench, close_workbench and present_art. Reopening with the same control_id reuses the instance. Also pass its workbench_id to present_art {control_id,workbench_id,view} and close_workbench. Share the URL with the human; keep control IDs private.
One controller per workspace; four workbenches per shared runtime directory. Never bypass control or limits with shell servers, PIDs or alternate paths. HTTP ends when its hosting MCP process exits or its returned idle timeout expires; polling does not renew activity. Close when finished, never while the human is working. Art and drafts persist.

HUMAN SCREEN
At first handoff, briefly explain available controls in the user's language: palette + click/drag paints; Mark issues: left adds, right removes; Undo once for the last stroke (Ctrl/Cmd+Z); Save (Ctrl/Cmd+S). For more than 20 allowed colors, explain that the screen shows common colors and opens in Mark issues mode. Invite the user to mark changes; read the saved concerns, focus those areas and make edits within the request's permissions. Clarify ambiguous intent. Offer older results through conversation. Repeat only when needed. Prepare collections, filters, past art, reference crops and known-time playback yourself.
Read inspect_presentation; copy current IDs into view.expected_presentation_id/expected_state_id (both explicit null only for the first view). saved.art_ids holds the explicit save; state.art_ids may contain a newer draft. state.concerns and saved.concerns identify marked candidates and pixels; marks request observation, not edit permission or rejection.

EDIT AND RECOVER
Read the request and target. Preserve palette order, duplicate indices, transparency, dimensions and constraints. validate_art locations and focus_art grant no write permission. Pass request_id for protection; preview sets before applying the exact plan and bindings.
Await dependent calls. Edit immutable candidates, compare, revalidate and submit_edit_result. After conflicts or lost responses, inspect control and durable state before retrying; never replay stale edits.
review_edit_result records only the user's expressed judgment. Saving, submission, validation, acceptance and game verification are separate; report only observed stages."#;
