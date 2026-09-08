use super::*;
use crate::{
    selection::{CreateSelection, PixelGuard, Selection, create_selection},
    workflow::*,
};

struct StoredRequest {
    payload: Value,
    input: RequestEdit,
    result_id: Option<String>,
    notes: Option<String>,
}
fn latest_review(database: &Connection, request_id: &str) -> ArtResult<Option<Value>> {
    let row: Option<(String, Vec<u8>)> = database
        .query_row(
            "SELECT id,payload FROM edit_reviews WHERE request_id=?1 ORDER BY seq DESC LIMIT 1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    row.map(|(id, bytes)| read_review(&id, &bytes)).transpose()
}
fn read_review(id: &str, bytes: &[u8]) -> ArtResult<Value> {
    let payload: Value = serde_json::from_slice(bytes).map_err(storage_error)?;
    if format!("review_{}", hash_json(&payload["input"])?) != id {
        return Err(ArtError::new(
            "integrity_error",
            "Review record hash does not match",
        ));
    }
    Ok(payload)
}
fn review_status(review: &Option<Value>) -> Value {
    review
        .as_ref()
        .map_or(json!("pending"), |v| v["input"]["decision"].clone())
}
fn review_id(review: &Option<Value>) -> Option<String> {
    review
        .as_ref()
        .and_then(|r| r["review_id"].as_str().map(str::to_owned))
}
fn insert_request(database: &Connection, id: &str, payload: &Value) -> ArtResult<()> {
    database
        .execute(
            "INSERT OR IGNORE INTO edit_requests(id,payload) VALUES (?1,?2)",
            params![id, encode_json(payload)?],
        )
        .map_err(storage_error)?;
    Ok(())
}
impl Workspace {
    fn review_by_id(&self, id: &str) -> ArtResult<Value> {
        let bytes: Option<Vec<u8>> = self
            .database
            .query_row("SELECT payload FROM edit_reviews WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(storage_error)?;
        read_review(
            id,
            &bytes.ok_or_else(|| invalid("Previous review was not found"))?,
        )
    }
    pub(super) fn load_selection(&self, id: &str) -> ArtResult<Selection> {
        let bytes: Option<Vec<u8>> = self
            .database
            .query_row("SELECT payload FROM selections WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(storage_error)?;
        let bytes = bytes.ok_or_else(|| {
            ArtError::new("selection_not_found", "Selection was not found")
                .detail(json!({"selection_id":id}))
        })?;
        let selection: Selection = serde_json::from_slice(&bytes).map_err(storage_error)?;
        if selection.id()? != id {
            return Err(ArtError::new(
                "integrity_error",
                "Selection data hash does not match",
            ));
        }
        Ok(selection)
    }
    pub(super) fn select_pixels(&mut self, input: CreateSelection) -> ArtResult<ToolOutput> {
        let art = self.load(&input.art_id)?;
        let selection = create_selection(&art, input)?;
        let id = selection.id()?;
        self.database
            .execute(
                "INSERT OR IGNORE INTO selections(id,payload) VALUES (?1,?2)",
                params![id, encode_json(&selection)?],
            )
            .map_err(storage_error)?;
        let mut output = ToolOutput::new(selection.summary()?);
        output.links.push(format!("dotmend://selections/{id}"));
        Ok(output)
    }
    fn load_request(&self, id: &str) -> ArtResult<StoredRequest> {
        let row: Option<(Vec<u8>, Option<String>, Option<String>)> = self
            .database
            .query_row(
                "SELECT payload,result_id,notes FROM edit_requests WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(storage_error)?;
        let (bytes, result_id, notes) = row.ok_or_else(|| {
            ArtError::new("request_not_found", "Edit request was not found")
                .detail(json!({"request_id":id}))
        })?;
        let payload: Value = serde_json::from_slice(&bytes).map_err(storage_error)?;
        if format!("request_{}", hash_json(&payload)?) != id {
            return Err(ArtError::new(
                "integrity_error",
                "Edit request hash does not match",
            ));
        }
        let mut input = payload.clone();
        input
            .as_object_mut()
            .ok_or_else(|| invalid("Invalid request format"))?
            .remove("target_hash");
        input
            .as_object_mut()
            .expect("request object")
            .remove("coordinate_system");
        Ok(StoredRequest {
            payload,
            input: decode(input)?,
            result_id,
            notes,
        })
    }
    fn request_guard(&self, id: &str, input: &RequestEdit) -> ArtResult<PixelGuard> {
        let base = self.load(&input.base_art_id)?;
        let write = input
            .write_selection_id
            .as_ref()
            .map(|id| self.load_selection(id))
            .transpose()?;
        let protected = input
            .protected_selection_ids
            .iter()
            .map(|id| self.load_selection(id))
            .collect::<ArtResult<Vec<_>>>()?;
        PixelGuard::new(
            id.into(),
            &base,
            input.write_region,
            write.as_ref(),
            &protected,
        )
    }
    pub(super) fn require_descendant(&self, result: &Art, base_id: &str) -> ArtResult<()> {
        let mut ancestors = vec![result.id()?];
        let mut seen = BTreeSet::new();
        while let Some(id) = ancestors.pop() {
            if id == base_id {
                return Ok(());
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            if seen.len() > 10000 {
                return Err(ArtError::new(
                    "limit_exceeded",
                    "Candidate ancestry exceeds the limit",
                ));
            }
            ancestors.extend(self.load(&id)?.parents);
        }
        Err(
            invalid("Candidate must descend from the request's base art")
                .detail(json!({"base_art_id":base_id,"result_art_id":result.id()?})),
        )
    }
    pub(super) fn candidate_guard(&self, id: &str, art: &Art) -> ArtResult<PixelGuard> {
        let stored = self.load_request(id)?;
        self.bound_guard(id, art, stored.input.write_region)
    }
    pub(super) fn bound_guard(&self, id: &str, art: &Art, region: Rect) -> ArtResult<PixelGuard> {
        let stored = self.load_request(id)?;
        region.check_within(stored.input.write_region)?;
        let base = self.load(&stored.input.base_art_id)?;
        self.require_descendant(art, &stored.input.base_art_id)?;
        let guard = self.request_guard(id, &stored.input)?;
        guard.check_preserved(&base, art)?;
        Ok(guard)
    }
    fn prepare_request(&self, input: &RequestEdit) -> ArtResult<(String, Value)> {
        self.prepare_request_with_review(input, None)
    }
    fn prepare_request_with_review(
        &self,
        input: &RequestEdit,
        pending_review: Option<&str>,
    ) -> ArtResult<(String, Value)> {
        let art = self.load(&input.base_art_id)?;
        input.write_region.check_within(art.target.bounds())?;
        if input.instruction.trim().is_empty() || input.instruction.len() > 4096 {
            return Err(invalid("Edit instruction must contain 1..4096 bytes"));
        }
        let mut payload = serde_json::to_value(input).map_err(storage_error)?;
        let mut context = payload.clone();
        for key in ["instruction", "base_art_id", "write_region"] {
            context.as_object_mut().expect("request object").remove(key);
        }
        if encode_json(&context)?.len() > 32 * 1024
            || input.protected_selection_ids.len() + usize::from(input.write_selection_id.is_some())
                > 32
            || input.reference_regions.len() > 32
            || input.related_art_ids.len() > MAX_SET_SIZE
        {
            return Err(ArtError::new(
                "limit_exceeded",
                "Request context, selections or related candidates exceed the limit",
            ));
        }
        if input
            .protected_selection_ids
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != input.protected_selection_ids.len()
            || input.related_art_ids.iter().collect::<BTreeSet<_>>().len()
                != input.related_art_ids.len()
        {
            return Err(invalid(
                "Selection or related candidate IDs contain duplicates",
            ));
        }
        self.request_guard("", input)?;
        for role in &input.palette_roles {
            if role.label.trim().is_empty() || role.indices.is_empty() {
                return Err(invalid("Palette roles require a name and indices"));
            }
            for &index in &role.indices {
                if index as usize >= art.target.palette.len() {
                    return Err(invalid("Palette role index is out of bounds"));
                }
            }
            if let Some(r) = &role.source_ref {
                check_source_ref(r)?;
            }
        }
        for region in &input.reference_regions {
            region.target_region.check_within(art.target.bounds())?;
            if !art
                .references
                .iter()
                .any(|r| r.source_hash == region.source_hash)
            {
                return Err(invalid("Reference image is not attached"));
            }
            let source = decode_png(&self.source(&region.source_hash)?)?;
            region.source_region.check_within(Rect {
                x: 0,
                y: 0,
                width: source.width,
                height: source.height,
            })?;
            if let Some(r) = &region.source_ref {
                check_source_ref(r)?;
            }
        }
        for id in &input.related_art_ids {
            self.load(id)?;
        }
        for source in &input.source_refs {
            check_source_ref(source)?;
        }
        if let Some(previous) = &input.previous_request_id {
            let prior = self.load_request(previous)?;
            if self.load(&prior.input.base_art_id)?.target != art.target {
                return Err(ArtError::new(
                    "target_mismatch",
                    "Follow-up target constraints differ from the previous request",
                ));
            }
        }
        if let Some(id) = &input.previous_review_id {
            if input.previous_request_id.is_none() {
                return Err(invalid("Specify both the previous review and its request"));
            }
            if pending_review != Some(id.as_str()) {
                let review = self.review_by_id(id)?;
                if review["input"]["request_id"].as_str() != input.previous_request_id.as_deref() {
                    return Err(invalid("Previous review belongs to a different request"));
                }
            }
        }
        if let Some(context) = &input.frame_context {
            if context
                .frames
                .iter()
                .any(|f| !input.related_art_ids.contains(&f.art_id))
            {
                return Err(invalid("Frames must be included in related_art_ids"));
            }
            let arts = context
                .frames
                .iter()
                .map(|f| self.load(&f.art_id))
                .collect::<ArtResult<Vec<_>>>()?;
            crate::frames::check_frames(&context.frames, &arts)?;
            if let Some(r) = &context.source_ref {
                check_source_ref(r)?;
            }
        }
        payload["target_hash"] = json!(hash_json(&art.target)?);
        payload["coordinate_system"] = json!("pixel_top_left_xy");
        Ok((format!("request_{}", hash_json(&payload)?), payload))
    }
    pub(super) fn request_edit(&mut self, input: RequestEdit) -> ArtResult<ToolOutput> {
        let (id, payload) = self.prepare_request(&input)?;
        insert_request(&self.database, &id, &payload)?;
        Ok(ToolOutput::new(json!({"request_id":id,"request":payload})))
    }
    pub(super) fn inspect_request(&self, input: InspectEditRequest) -> ArtResult<ToolOutput> {
        let limit = input.limit.unwrap_or(30);
        if limit == 0
            || limit > 100
            || input.review_cursor.is_some_and(|c| c < 0)
            || input.follow_up_cursor.is_some_and(|c| c < 0)
        {
            return Err(invalid(
                "limit must be 1..100 and cursor must be nonnegative",
            ));
        }
        let _snapshot = self
            .database
            .unchecked_transaction()
            .map_err(storage_error)?;
        let stored = self.load_request(&input.request_id)?;
        let art = self.load(&stored.input.base_art_id)?;
        let mut selections = vec![];
        let mut links = vec![];
        for id in stored
            .input
            .write_selection_id
            .iter()
            .chain(&stored.input.protected_selection_ids)
        {
            selections.push(self.load_selection(id)?.summary()?);
            links.push(format!("dotmend://selections/{id}"));
        }
        links.extend(
            art.references
                .iter()
                .map(|r| format!("dotmend://sources/{}", r.source_hash)),
        );
        let related = stored
            .input
            .related_art_ids
            .iter()
            .map(|id| self.load(id)?.summary())
            .collect::<ArtResult<Vec<_>>>()?;
        let mut stmt=self.database.prepare("SELECT seq,id,payload FROM edit_reviews WHERE request_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3").map_err(storage_error)?;
        let rows = stmt
            .query_map(
                params![
                    input.request_id,
                    input.review_cursor.unwrap_or(0),
                    limit + 1
                ],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        let reviews = rows
            .iter()
            .take(limit)
            .map(|(_, id, bytes)| read_review(id, bytes))
            .collect::<ArtResult<Vec<_>>>()?;
        let review_cursor = (rows.len() > limit).then(|| rows[limit - 1].0);
        let mut stmt=self.database.prepare("SELECT seq,id FROM edit_requests WHERE json_extract(CAST(payload AS TEXT),'$.previous_request_id')=?1 AND seq>?2 ORDER BY seq LIMIT ?3").map_err(storage_error)?;
        let rows = stmt
            .query_map(
                params![
                    input.request_id,
                    input.follow_up_cursor.unwrap_or(0),
                    limit + 1
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        let follow_ups = rows
            .iter()
            .take(limit)
            .map(|(_, id)| self.request_entry(id))
            .collect::<ArtResult<Vec<_>>>()?;
        let follow_up_cursor = (rows.len() > limit).then(|| rows[limit - 1].0);
        let review = latest_review(&self.database, &input.request_id)?;
        let previous_request = stored
            .input
            .previous_request_id
            .as_ref()
            .map(|id| self.request_entry(id))
            .transpose()?;
        let previous_review = stored
            .input
            .previous_review_id
            .as_ref()
            .map(|id| self.review_by_id(id))
            .transpose()?;
        let mut output = ToolOutput::new(
            json!({"request_id":input.request_id,"request":stored.payload,"target":art.target,"previous_request":previous_request,"previous_review":previous_review,"selections":selections,"references":art.references,"related_arts":related,"result_art_id":stored.result_id,"notes":stored.notes,"status":if stored.result_id.is_some(){"submitted"}else{"pending"},"human_review":review_status(&review),"current_review_id":review_id(&review),"latest_review":review,"reviews":reviews,"next_review_cursor":review_cursor,"follow_up_requests":follow_ups,"next_follow_up_cursor":follow_up_cursor}),
        );
        output.links = links;
        Ok(output)
    }
    fn request_entry(&self, id: &str) -> ArtResult<Value> {
        let stored = self.load_request(id)?;
        let review = latest_review(&self.database, id)?;
        Ok(
            json!({"request_id":id,"request":stored.payload,"result_art_id":stored.result_id,"notes":stored.notes,"status":if stored.result_id.is_some(){"submitted"}else{"pending"},"human_review":review_status(&review),"current_review_id":review_id(&review)}),
        )
    }
    pub(super) fn list_requests(&self, input: ListEditRequests) -> ArtResult<ToolOutput> {
        if input
            .status
            .as_deref()
            .is_some_and(|s| !["pending", "submitted"].contains(&s))
        {
            return Err(invalid("status must be pending or submitted"));
        }
        let limit = input.limit.unwrap_or(50);
        if limit == 0 || limit > 100 || input.cursor.is_some_and(|c| c < 0) {
            return Err(invalid(
                "limit must be 1..100 and cursor must be nonnegative",
            ));
        }
        let _snapshot = self
            .database
            .unchecked_transaction()
            .map_err(storage_error)?;
        let mut stmt=self.database.prepare("SELECT seq,id FROM edit_requests WHERE seq>?1 AND (?2 IS NULL OR (?2='pending' AND result_id IS NULL) OR (?2='submitted' AND result_id IS NOT NULL)) ORDER BY seq LIMIT ?3").map_err(storage_error)?;
        let rows = stmt
            .query_map(
                params![input.cursor.unwrap_or(0), input.status, limit + 1],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        let entries = rows
            .iter()
            .take(limit)
            .map(|(_, id)| self.request_entry(id))
            .collect::<ArtResult<Vec<_>>>()?;
        Ok(ToolOutput::new(
            json!({"requests":entries,"next_cursor":(rows.len()>limit).then(||rows[limit-1].0)}),
        ))
    }
    pub(super) fn submit_result(&mut self, input: SubmitEditResult) -> ArtResult<ToolOutput> {
        if input.notes.len() > 4096 {
            return Err(invalid("Result notes exceed the size limit"));
        }
        let stored = self.load_request(&input.request_id)?;
        let base = self.load(&stored.input.base_art_id)?;
        let result = self.load(&input.result_art_id)?;
        self.require_descendant(&result, &stored.input.base_art_id)?;
        self.request_guard(&input.request_id, &stored.input)?
            .check_preserved(&base, &result)?;
        let transaction = self
            .database
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let (previous, notes): (Option<String>, Option<String>) = transaction
            .query_row(
                "SELECT result_id,notes FROM edit_requests WHERE id=?1",
                [&input.request_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(storage_error)?;
        if let Some(previous) = previous {
            if previous != input.result_art_id || notes.as_deref() != Some(&input.notes) {
                return Err(ArtError::new(
                    "submission_conflict",
                    "A different result has already been submitted",
                )
                .detail(json!({"request_id":input.request_id,"result_art_id":previous})));
            }
        } else {
            transaction
                .execute(
                    "UPDATE edit_requests SET result_id=?1,notes=?2 WHERE id=?3",
                    params![input.result_art_id, input.notes, input.request_id],
                )
                .map_err(storage_error)?;
        }
        let review = latest_review(&transaction, &input.request_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(ToolOutput::new(
            json!({"request_id":input.request_id,"result_art_id":input.result_art_id,"status":"submitted","human_review":review_status(&review),"current_review_id":review_id(&review),"diff":edit_summary(&base,&result)?}),
        ))
    }
    pub fn review_result(&mut self, mut input: ReviewEditResult) -> ArtResult<ToolOutput> {
        if input.regions.len() > 32
            || input.notes.len() + input.regions.iter().map(|r| r.comment.len()).sum::<usize>()
                > 4096
        {
            return Err(ArtError::new(
                "limit_exceeded",
                "Review notes or regions exceed the limit",
            ));
        }
        let stored = self.load_request(&input.request_id)?;
        if stored.result_id.as_deref() != Some(&input.result_art_id) {
            return Err(ArtError::new(
                "review_conflict",
                "Review candidate differs from the submitted candidate",
            )
            .detail(json!({"result_art_id":stored.result_id})));
        }
        let result = self.load(&input.result_art_id)?;
        for region in &input.regions {
            region.region.check_within(result.target.bounds())?;
            if region.comment.trim().is_empty() {
                return Err(invalid("Region feedback must not be empty"));
            }
        }
        if let Some(next) = &mut input.follow_up {
            if input.decision != ReviewDecision::ChangesRequested {
                return Err(invalid(
                    "A follow-up request can only accompany a changes_requested review",
                ));
            }
            if next
                .previous_request_id
                .as_deref()
                .is_some_and(|id| id != input.request_id)
            {
                return Err(invalid("Follow-up previous_request_id does not match"));
            }
            next.previous_request_id = Some(input.request_id.clone());
            if next.previous_review_id.is_some() {
                return Err(invalid(
                    "The server assigns previous_review_id when saving a follow-up with its review",
                ));
            }
        }
        let id = format!("review_{}", hash_json(&json!(input))?);
        let follow = input
            .follow_up
            .as_ref()
            .map(|next| {
                let mut next = next.clone();
                next.previous_review_id = Some(id.clone());
                self.prepare_request_with_review(&next, Some(&id))
            })
            .transpose()?;
        let payload = json!({"review_id":id,"input":input,"follow_up_request_id":follow.as_ref().map(|(id,_)|id)});
        let transaction = self
            .database
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let existing: Option<Vec<u8>> = transaction
            .query_row("SELECT payload FROM edit_reviews WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(storage_error)?;
        let current = latest_review(&transaction, &input.request_id)?;
        let record = if let Some(bytes) = existing {
            read_review(&id, &bytes)?
        } else {
            if review_id(&current) != input.expected_review_id {
                return Err(ArtError::new("review_conflict","A newer review has been saved since the review you read").detail(json!({"request_id":input.request_id,"current_review_id":review_id(&current),"human_review":review_status(&current)})));
            }
            if let Some((id, payload)) = &follow {
                insert_request(&transaction, id, payload)?;
            }
            transaction
                .execute(
                    "INSERT INTO edit_reviews(id,request_id,payload) VALUES (?1,?2,?3)",
                    params![id, input.request_id, encode_json(&payload)?],
                )
                .map_err(storage_error)?;
            payload
        };
        let current = latest_review(&transaction, &input.request_id)?;
        transaction.commit().map_err(storage_error)?;
        Ok(ToolOutput::new(
            json!({"review":record,"current_review_id":review_id(&current),"human_review":review_status(&current)}),
        ))
    }
}
