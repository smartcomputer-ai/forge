use super::*;

impl Session {
    pub(super) fn next_persistence_sequence(&mut self) -> u64 {
        let current = self.persistence_sequence_no;
        self.persistence_sequence_no = self.persistence_sequence_no.saturating_add(1);
        current
    }

    pub(super) fn persistence_enabled(&self) -> bool {
        self.persistence_writer.is_some() && self.persistence_mode != CxdbPersistenceMode::Off
    }

    pub async fn persistence_snapshot(&mut self) -> Result<SessionPersistenceSnapshot, AgentError> {
        let mut snapshot = SessionPersistenceSnapshot {
            session_id: self.id.clone(),
            context_id: self.persistence_context_id.clone(),
            head_turn_id: None,
        };

        if !self.persistence_enabled() {
            return Ok(snapshot);
        }

        self.ensure_persistence_context().await?;
        snapshot.context_id = self.persistence_context_id.clone();

        if let (Some(store), Some(context_id)) = (
            self.persistence_writer.clone(),
            self.persistence_context_id.clone(),
        ) {
            match store.get_head(&context_id).await {
                Ok(head) => snapshot.head_turn_id = Some(head.turn_id),
                Err(error) => self.handle_persistence_error(error, "get_head")?,
            }
        }

        Ok(snapshot)
    }

    pub(super) fn persist_session_event_blocking(
        &mut self,
        event_kind: &str,
        payload: Value,
    ) -> Result<(), AgentError> {
        if !self.persistence_enabled() {
            return Ok(());
        }
        let Some(store) = self.persistence_writer.clone() else {
            return Ok(());
        };

        if self.persistence_context_id.is_none() {
            let created = run_cxdb_future_blocking("create_context", {
                let store = store.clone();
                async move { store.create_context(None).await }
            });
            match created {
                Ok(context) => {
                    let head_turn_id = if context.head_turn_id == "0" {
                        None
                    } else {
                        Some(context.head_turn_id)
                    };
                    self.persistence_context_id = Some(context.context_id);
                    self.persistence_parent_turn_id = head_turn_id;
                }
                Err(error) => return self.handle_persistence_error(error, "create_context"),
            }
        }

        let Some(context_id) = self.persistence_context_id.clone() else {
            return Ok(());
        };

        let snapshot_capture = match capture_fs_snapshot_blocking(
            store.clone(),
            self.config.fs_snapshot_policy.as_ref(),
            self.execution_env.working_directory(),
        ) {
            Ok(value) => value,
            Err(error) => return self.handle_persistence_error(error, "capture_upload_workspace"),
        };

        let sequence_no = self.next_persistence_sequence();
        let kind = match event_kind {
            "session_start" => "started",
            "session_end" => "ended",
            other => other,
        };
        let final_state = payload
            .get("final_state")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let (fs_root_hash, snapshot_policy_id, snapshot_stats) =
            snapshot_capture_fields(snapshot_capture.as_ref());
        let record = SessionLifecycleRecord {
            session_id: self.id.clone(),
            kind: kind.to_string(),
            timestamp: current_timestamp(),
            final_state,
            sequence_no,
            thread_key: self.thread_key.clone(),
            fs_root_hash,
            snapshot_policy_id,
            snapshot_stats,
        };
        let payload_bytes = encode_typed_record("forge.agent.session_lifecycle", &record)?;
        let idempotency_key = agent_idempotency_key(&self.id, sequence_no, event_kind);
        let request = CxdbAppendTurnRequest {
            context_id,
            parent_turn_id: self.persistence_parent_turn_id.clone(),
            type_id: "forge.agent.session_lifecycle".to_string(),
            type_version: agent_type_version("forge.agent.session_lifecycle"),
            payload: payload_bytes,
            idempotency_key,
            fs_root_hash: snapshot_capture
                .as_ref()
                .map(|capture| capture.fs_root_hash.clone()),
        };

        match run_cxdb_future_blocking("append_turn", {
            let store = store.clone();
            async move { store.append_turn(request).await }
        }) {
            Ok(turn) => {
                self.persistence_parent_turn_id = Some(turn.turn_id);
                Ok(())
            }
            Err(error) => self.handle_persistence_error(error, "append_turn"),
        }
    }

    pub(super) fn handle_persistence_error(
        &self,
        error: CxdbClientError,
        operation: &str,
    ) -> Result<(), AgentError> {
        match self.persistence_mode {
            CxdbPersistenceMode::Off => Ok(()),
            CxdbPersistenceMode::Required => {
                Err(SessionError::Persistence(format!("{} failed: {}", operation, error)).into())
            }
        }
    }

    pub(super) async fn ensure_persistence_context(&mut self) -> Result<(), AgentError> {
        if !self.persistence_enabled() || self.persistence_context_id.is_some() {
            return Ok(());
        }
        let Some(store) = self.persistence_writer.clone() else {
            return Ok(());
        };
        match store.create_context(None).await {
            Ok(context) => {
                self.persistence_parent_turn_id = if context.head_turn_id == "0" {
                    None
                } else {
                    Some(context.head_turn_id.clone())
                };
                self.persistence_context_id = Some(context.context_id);
                Ok(())
            }
            Err(error) => self.handle_persistence_error(error, "create_context"),
        }
    }

    pub(super) async fn persist_turn_if_enabled(&mut self, turn: &Turn) -> Result<(), AgentError> {
        if !self.persistence_enabled() {
            return Ok(());
        }

        let (type_id, timestamp, turn_payload) = match turn {
            Turn::User(turn) => (
                "forge.agent.user_turn",
                turn.timestamp.clone(),
                serde_json::to_value(turn)
                    .map_err(|err| SessionError::Persistence(err.to_string()))?,
            ),
            Turn::Assistant(turn) => (
                "forge.agent.assistant_turn",
                turn.timestamp.clone(),
                serde_json::to_value(turn)
                    .map_err(|err| SessionError::Persistence(err.to_string()))?,
            ),
            Turn::ToolResults(turn) => (
                "forge.agent.tool_results_turn",
                turn.timestamp.clone(),
                serde_json::to_value(turn)
                    .map_err(|err| SessionError::Persistence(err.to_string()))?,
            ),
            Turn::System(turn) => (
                "forge.agent.system_turn",
                turn.timestamp.clone(),
                serde_json::to_value(turn)
                    .map_err(|err| SessionError::Persistence(err.to_string()))?,
            ),
            Turn::Steering(turn) => (
                "forge.agent.steering_turn",
                turn.timestamp.clone(),
                serde_json::to_value(turn)
                    .map_err(|err| SessionError::Persistence(err.to_string()))?,
            ),
        };

        self.persist_typed_payload(
            type_id,
            "turn_appended",
            AgentTurnRecord {
                session_id: self.id.clone(),
                timestamp,
                turn: turn_payload,
                sequence_no: 0,
                thread_key: self.thread_key.clone(),
                fs_root_hash: None,
                snapshot_policy_id: None,
                snapshot_stats: None,
            },
        )
        .await
    }

    pub(super) async fn persist_event_turn(
        &mut self,
        event_kind: &str,
        payload: Value,
    ) -> Result<(), AgentError> {
        let (call_id, tool_name, arguments, output, is_error, kind) = match event_kind {
            "tool_call_start" => (
                payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                payload
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                payload.get("arguments").cloned(),
                None,
                None,
                "started".to_string(),
            ),
            "tool_call_end" => (
                payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                None,
                None,
                payload.get("output").cloned(),
                payload.get("is_error").and_then(Value::as_bool),
                "ended".to_string(),
            ),
            _ => {
                return Err(SessionError::Persistence(format!(
                    "unsupported event kind for typed lifecycle persistence: {event_kind}"
                ))
                .into());
            }
        };
        self.persist_typed_payload(
            "forge.agent.tool_call_lifecycle",
            event_kind,
            ToolCallLifecycleRecord {
                session_id: self.id.clone(),
                kind,
                timestamp: current_timestamp(),
                call_id,
                tool_name,
                arguments,
                output,
                is_error,
                sequence_no: 0,
                thread_key: self.thread_key.clone(),
                fs_root_hash: None,
                snapshot_policy_id: None,
                snapshot_stats: None,
            },
        )
        .await
    }

    pub(super) async fn persist_typed_payload<T: Serialize + DeserializeOwned>(
        &mut self,
        type_id: &str,
        event_kind: &str,
        mut record: T,
    ) -> Result<(), AgentError> {
        if !self.persistence_enabled() {
            return Ok(());
        }
        self.ensure_persistence_context().await?;
        let Some(store) = self.persistence_writer.clone() else {
            return Ok(());
        };
        let Some(context_id) = self.persistence_context_id.clone() else {
            return Ok(());
        };

        let snapshot_capture = if let Some(policy) = self.config.fs_snapshot_policy.as_ref() {
            let workspace_root = self.execution_env.working_directory();
            match store.capture_upload_workspace(workspace_root, policy).await {
                Ok(capture) => Some(capture),
                Err(error) => {
                    return self.handle_persistence_error(error, "capture_upload_workspace");
                }
            }
        } else {
            None
        };

        let sequence_no = self.next_persistence_sequence();
        apply_sequence_and_fs_to_record(
            &mut record,
            sequence_no,
            self.thread_key.clone(),
            snapshot_capture.as_ref(),
        )?;
        let payload_bytes = encode_typed_record(type_id, &record)?;
        let idempotency_key = agent_idempotency_key(&self.id, sequence_no, event_kind);
        let request = CxdbAppendTurnRequest {
            context_id,
            parent_turn_id: self.persistence_parent_turn_id.clone(),
            type_id: type_id.to_string(),
            type_version: agent_type_version(type_id),
            payload: payload_bytes,
            idempotency_key,
            fs_root_hash: snapshot_capture
                .as_ref()
                .map(|capture| capture.fs_root_hash.clone()),
        };
        match store.append_turn(request).await {
            Ok(turn) => {
                self.persistence_parent_turn_id = Some(turn.turn_id);
                Ok(())
            }
            Err(error) => self.handle_persistence_error(error, "append_turn"),
        }
    }
}
