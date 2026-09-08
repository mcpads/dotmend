use super::*;
use crate::presentation::*;

fn identity(prefix: &str, value: &impl Serialize) -> ArtResult<String> {
    Ok(format!("{prefix}_{}", hash_json(value)?))
}
fn active(database: &Connection) -> ArtResult<Option<(String, String)>> {
    database.query_row("SELECT p.id,p.head_id FROM active_presentation a JOIN presentations p ON p.id=a.id WHERE a.singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage_error)
}
fn check_active(
    database: &Connection,
    presentation: Option<&str>,
    state: Option<&str>,
) -> ArtResult<()> {
    let current = active(database)?;
    if current.as_ref().map(|v| v.0.as_str()) != presentation
        || current.as_ref().map(|v| v.1.as_str()) != state
    {
        return Err(ArtError::new("presentation_conflict","The presentation has changed. Inspect the current presentation before retrying").detail(json!({"presentation_id":current.as_ref().map(|v|&v.0),"state_id":current.as_ref().map(|v|&v.1)})));
    }
    Ok(())
}
fn insert_state(database: &Connection, state: &PresentationState) -> ArtResult<String> {
    let id = identity("view_state", state)?;
    database
        .execute(
            "INSERT OR IGNORE INTO presentation_states(id,payload) VALUES (?1,?2)",
            params![id, encode_json(state)?],
        )
        .map_err(storage_error)?;
    Ok(id)
}
impl Workspace {
    fn presentation_state(&self, id: &str) -> ArtResult<PresentationState> {
        let bytes: Option<Vec<u8>> = self
            .database
            .query_row(
                "SELECT payload FROM presentation_states WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage_error)?;
        let value: PresentationState = serde_json::from_slice(&bytes.ok_or_else(|| {
            ArtError::new(
                "presentation_not_found",
                "Stored presentation state was not found",
            )
        })?)
        .map_err(storage_error)?;
        if identity("view_state", &value)? != id {
            return Err(ArtError::new(
                "integrity_error",
                "Presentation state hash does not match",
            ));
        }
        Ok(value)
    }
    fn presentation_snapshot(
        &self,
        id: &str,
        state_id: Option<&str>,
    ) -> ArtResult<PresentationSnapshot> {
        let row: Option<(Vec<u8>, String, Option<String>)> = self
            .database
            .query_row(
                "SELECT payload,head_id,saved_state_id FROM presentations WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(storage_error)?;
        let (bytes, head, saved_id) = row.ok_or_else(|| {
            ArtError::new(
                "presentation_not_found",
                "Stored presentation was not found",
            )
        })?;
        let presentation: PresentArt = serde_json::from_slice(&bytes).map_err(storage_error)?;
        if identity("presentation", &presentation)? != id {
            return Err(ArtError::new(
                "integrity_error",
                "Presentation configuration hash does not match",
            ));
        }
        let state_id = state_id.unwrap_or(&head).to_owned();
        let state = self.presentation_state(&state_id)?;
        if state.presentation_id != id {
            return Err(invalid("State belongs to a different presentation"));
        }
        let saved = if let Some(saved_id) = saved_id {
            let state = self.presentation_state(&saved_id)?;
            Some(SavedPresentation {
                save_id: identity("save", &json!({"presentation_id":id,"state_id":saved_id}))?,
                presentation_id: id.into(),
                state_id: saved_id,
                art_ids: state.art_ids,
                concerns: state.concerns,
            })
        } else {
            None
        };
        Ok(PresentationSnapshot {
            is_current: active(&self.database)?.is_some_and(|v| v.0 == id && v.1 == state_id),
            presentation_id: id.into(),
            presentation,
            state_id: state_id.clone(),
            current_state_id: head,
            dirty: saved.as_ref().is_none_or(|s| s.state_id != state_id),
            undo_available: state.undo_art_ids.is_some(),
            state,
            saved,
        })
    }
    pub fn inspect_presentation(&self, input: InspectPresentation) -> ArtResult<ToolOutput> {
        let _snapshot = self
            .database
            .unchecked_transaction()
            .map_err(storage_error)?;
        let id = input
            .presentation_id
            .or(active(&self.database)?.map(|v| v.0));
        let view = id
            .map(|id| self.presentation_snapshot(&id, input.state_id.as_deref()))
            .transpose()?;
        Ok(ToolOutput::new(json!({"presentation":view})))
    }
    pub(super) fn present_art(&mut self, input: PresentArt) -> ArtResult<ToolOutput> {
        if input.title.trim().is_empty()
            || input.title.len() > 512
            || input.note.len() > 4096
            || input.items.is_empty()
            || input.items.len() > MAX_SET_SIZE
        {
            return Err(invalid("Provide a presentation title and 1..16 items"));
        }
        let mut pixels = 0u64;
        let mut editable = BTreeSet::new();
        for item in &input.items {
            if item.label().trim().is_empty()
                || item.label().len() > 512
                || !(1..=64).contains(&item.scale())
            {
                return Err(invalid("Check item labels and scale (1..64)"));
            }
            let art = self.load(item.art_id())?;
            match item {
                PresentationItem::Art {
                    editable: can_edit,
                    request_id,
                    playback,
                    ..
                } => {
                    item.region().check_within(art.target.bounds())?;
                    if *can_edit && !editable.insert(item.art_id()) {
                        return Err(invalid(
                            "The same candidate cannot appear in two editable items",
                        ));
                    }
                    if let Some(id) = request_id {
                        self.candidate_guard(id, &art)?;
                    }
                    if let Some(frames) = playback {
                        if *can_edit
                            || item.region() != art.target.bounds()
                            || frames.first().is_none_or(|f| f.art_id != item.art_id())
                            || frames.iter().any(|f| f.duration_ms.is_none())
                        {
                            return Err(invalid(
                                "Playback requires a read-only item showing the full first candidate and a duration for every frame",
                            ));
                        }
                        let arts = frames
                            .iter()
                            .map(|f| self.load(&f.art_id))
                            .collect::<ArtResult<Vec<_>>>()?;
                        let rendered = crate::frames::render_frames(
                            &arts,
                            &crate::workflow::RenderArtSet {
                                frames: frames.clone(),
                                scale: item.scale(),
                                view: crate::workflow::SetView::Frames,
                            },
                        )?;
                        pixels += rendered.data["display_pixels"]
                            .as_u64()
                            .expect("frame display area");
                        continue;
                    }
                }
                PresentationItem::Reference { source_hash, .. } => {
                    if !art.references.iter().any(|r| r.source_hash == *source_hash) {
                        return Err(invalid("Reference is not attached to the candidate"));
                    }
                    let source = decode_png(&self.source(source_hash)?)?;
                    item.region().check_within(Rect {
                        x: 0,
                        y: 0,
                        width: source.width,
                        height: source.height,
                    })?;
                }
            }
            pixels += item.region().width as u64
                * item.region().height as u64
                * (item.scale() as u64).pow(2);
        }
        if pixels > MAX_PREVIEW_PIXELS as u64 {
            return Err(ArtError::new(
                "limit_exceeded",
                "Combined presentation display area exceeds the limit",
            )
            .detail(json!({"display_pixels":pixels,"max_preview_pixels":MAX_PREVIEW_PIXELS})));
        }
        let id = identity("presentation", &input)?;
        let state = PresentationState {
            presentation_id: id.clone(),
            art_ids: input.items.iter().map(|i| i.art_id().into()).collect(),
            previous_state_id: None,
            undo_art_ids: None,
            action: PresentationActionKind::Open,
            concerns: vec![],
            undo_concerns: None,
        };
        let transaction = self
            .database
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM presentations WHERE id=?1)",
                [&id],
                |r| r.get(0),
            )
            .map_err(storage_error)?;
        if !exists {
            check_active(
                &transaction,
                input.expected_presentation_id.as_deref(),
                input.expected_state_id.as_deref(),
            )?;
            let state_id = insert_state(&transaction, &state)?;
            transaction
                .execute(
                    "INSERT INTO presentations(id,payload,head_id) VALUES (?1,?2,?3)",
                    params![id, encode_json(&input)?, state_id],
                )
                .map_err(storage_error)?;
            transaction.execute("INSERT INTO active_presentation(singleton,id) VALUES (1,?1) ON CONFLICT(singleton) DO UPDATE SET id=excluded.id",[&id]).map_err(storage_error)?;
        }
        transaction.commit().map_err(storage_error)?;
        self.inspect_presentation(InspectPresentation {
            presentation_id: Some(id),
            state_id: None,
        })
    }
    pub fn human_action(&mut self, input: HumanAction) -> ArtResult<ToolOutput> {
        let (presentation_id, expected_state_id) = input.identity();
        let command_id = identity("view_action", &input)?;
        let completed: Option<String> = self
            .database
            .query_row(
                "SELECT state_id FROM presentation_actions WHERE id=?1",
                [&command_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage_error)?;
        if let Some(state) = completed {
            return self.action_receipt(state, matches!(input, HumanAction::Save { .. }));
        }
        let snapshot = self.presentation_snapshot(presentation_id, Some(expected_state_id))?;
        let mut state = snapshot.state.clone();
        let mut arts = vec![];
        let save = matches!(input, HumanAction::Save { .. });
        match &input {
            HumanAction::Mark {
                item_index,
                pixels,
                marked,
                ..
            } => {
                let Some(PresentationItem::Art {
                    region,
                    playback: None,
                    ..
                }) = snapshot.presentation.items.get(*item_index)
                else {
                    return Err(invalid("Only static art items can be marked"));
                };
                if pixels.is_empty() || pixels.len() > MAX_CONCERN_PIXELS {
                    return Err(ArtError::new(
                        "limit_exceeded",
                        "Provide 1..4096 pixels per marking stroke",
                    ));
                }
                let previous: BTreeSet<_> = state
                    .concerns
                    .iter()
                    .find(|c| c.item_index == *item_index)
                    .into_iter()
                    .flat_map(|c| &c.pixels)
                    .map(|p| (p.y, p.x))
                    .collect();
                let mut selected = previous.clone();
                for pixel in pixels {
                    Rect {
                        x: pixel.x,
                        y: pixel.y,
                        width: 1,
                        height: 1,
                    }
                    .check_within(*region)?;
                    if *marked {
                        selected.insert((pixel.y, pixel.x));
                    } else {
                        selected.remove(&(pixel.y, pixel.x));
                    }
                }
                let other_count: usize = state
                    .concerns
                    .iter()
                    .filter(|c| c.item_index != *item_index)
                    .map(|c| c.pixels.len())
                    .sum();
                if other_count + selected.len() > MAX_CONCERN_PIXELS {
                    return Err(ArtError::new(
                        "limit_exceeded",
                        "A presentation can contain at most 4096 marked pixels",
                    ));
                }
                if selected != previous {
                    state.undo_art_ids = Some(state.art_ids.clone());
                    state.undo_concerns = Some(state.concerns.clone());
                    state.concerns.retain(|c| c.item_index != *item_index);
                    if !selected.is_empty() {
                        let mut bounds = None;
                        let pixels = selected
                            .into_iter()
                            .map(|(y, x)| {
                                crate::validation::include_point(&mut bounds, x, y);
                                crate::selection::Point { x, y }
                            })
                            .collect();
                        state.concerns.push(Concern {
                            item_index: *item_index,
                            art_id: state.art_ids[*item_index].clone(),
                            bounds: bounds.expect("nonempty concern pixels"),
                            pixels,
                        });
                        state.concerns.sort_by_key(|c| c.item_index);
                    }
                    state.previous_state_id = Some(expected_state_id.into());
                    state.action = PresentationActionKind::Mark;
                }
            }
            HumanAction::Paint {
                item_index, pixels, ..
            } => {
                let item = snapshot
                    .presentation
                    .items
                    .get(*item_index)
                    .ok_or_else(|| invalid("Presentation item was not found"))?;
                let PresentationItem::Art {
                    editable: true,
                    request_id,
                    region,
                    ..
                } = item
                else {
                    return Err(invalid("This art item is read-only"));
                };
                let base = self.load(&state.art_ids[*item_index])?;
                let guard = request_id
                    .as_ref()
                    .map(|id| self.candidate_guard(id, &base))
                    .transpose()?;
                let result = apply_guarded_edits(
                    &base,
                    *region,
                    &[EditOperation::SetPixels {
                        pixels: pixels.clone(),
                    }],
                    |id| self.load(id),
                    guard.as_ref(),
                )?;
                if result.id()? != state.art_ids[*item_index] {
                    state.undo_art_ids = Some(state.art_ids.clone());
                    state.undo_concerns = Some(state.concerns.clone());
                    state.art_ids[*item_index] = result.id()?;
                    for concern in &mut state.concerns {
                        if concern.item_index == *item_index {
                            // Carry the coordinates within this view; earlier states keep their own candidate IDs.
                            concern.art_id = state.art_ids[*item_index].clone();
                        }
                    }
                    state.previous_state_id = Some(expected_state_id.into());
                    state.action = PresentationActionKind::Paint;
                    arts.push(result);
                }
            }
            HumanAction::Undo { .. } => {
                state.art_ids = state
                    .undo_art_ids
                    .take()
                    .ok_or_else(|| invalid("There is no edit to undo"))?;
                state.concerns = state.undo_concerns.take().unwrap_or_default();
                state.previous_state_id = Some(expected_state_id.into());
                state.action = PresentationActionKind::Undo;
            }
            HumanAction::Save { .. } => {}
        }
        let transaction = self
            .database
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        // Other clients may have completed the same command while the candidate was prepared.
        let completed: Option<String> = transaction
            .query_row(
                "SELECT state_id FROM presentation_actions WHERE id=?1",
                [&command_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage_error)?;
        let state_id = if let Some(id) = completed {
            id
        } else {
            check_active(&transaction, Some(presentation_id), Some(expected_state_id))?;
            insert_arts(&transaction, &arts)?;
            let state_id = insert_state(&transaction, &state)?;
            if save {
                let saved = SavedPresentation {
                    save_id: identity(
                        "save",
                        &json!({"presentation_id":presentation_id,"state_id":state_id}),
                    )?,
                    presentation_id: presentation_id.into(),
                    state_id: state_id.clone(),
                    art_ids: state.art_ids,
                    concerns: state.concerns,
                };
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO presentation_saves(id,payload) VALUES (?1,?2)",
                        params![saved.save_id, encode_json(&saved)?],
                    )
                    .map_err(storage_error)?;
                transaction
                    .execute(
                        "UPDATE presentations SET saved_state_id=?1 WHERE id=?2",
                        params![state_id, presentation_id],
                    )
                    .map_err(storage_error)?;
            } else {
                transaction
                    .execute(
                        "UPDATE presentations SET head_id=?1 WHERE id=?2",
                        params![state_id, presentation_id],
                    )
                    .map_err(storage_error)?;
            }
            transaction
                .execute(
                    "INSERT INTO presentation_actions(id,state_id) VALUES (?1,?2)",
                    params![command_id, state_id],
                )
                .map_err(storage_error)?;
            state_id
        };
        transaction.commit().map_err(storage_error)?;
        self.action_receipt(state_id, save)
    }
    fn action_receipt(&self, state_id: String, saved: bool) -> ArtResult<ToolOutput> {
        let mut output = self.inspect_presentation(InspectPresentation::default())?;
        output.data["completed_state_id"] = json!(state_id);
        output.data["saved"] = json!(saved);
        Ok(output)
    }
}
