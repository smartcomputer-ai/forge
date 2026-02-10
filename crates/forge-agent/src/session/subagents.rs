use super::*;

impl Session {
    pub(super) async fn execute_subagent_tool_call(
        &mut self,
        tool_call: ToolCall,
    ) -> Result<ToolResult, AgentError> {
        let start_time = std::time::Instant::now();
        let arguments = parse_tool_call_arguments(&tool_call)?;
        self.event_emitter.emit(SessionEvent::tool_call_start(
            self.id.clone(),
            tool_call.name.clone(),
            tool_call.id.clone(),
            Some(arguments.clone()),
        ))?;

        if let Some(hook) = &self.tool_call_hook {
            let hook_context = crate::ToolHookContext {
                session_id: self.id.clone(),
                call_id: tool_call.id.clone(),
                tool_name: tool_call.name.clone(),
                arguments: arguments.clone(),
            };
            match hook.before_tool_call(&hook_context).await {
                Ok(crate::ToolPreHookOutcome::Continue) => {}
                Ok(crate::ToolPreHookOutcome::Skip { message, is_error }) => {
                    let duration_ms = start_time.elapsed().as_millis();
                    self.event_emitter.emit(SessionEvent::warning(
                        self.id.clone(),
                        format!("tool pre-hook skipped '{}': {}", tool_call.name, message),
                    ))?;
                    self.event_emitter.emit(SessionEvent::tool_call_end(
                        self.id.clone(),
                        tool_call.id.clone(),
                        if is_error {
                            None
                        } else {
                            Some(message.clone())
                        },
                        if is_error {
                            Some(message.clone())
                        } else {
                            None
                        },
                        duration_ms,
                        is_error,
                    ))?;
                    return Ok(ToolResult {
                        tool_call_id: tool_call.id,
                        content: Value::String(message),
                        is_error,
                    });
                }
                Ok(crate::ToolPreHookOutcome::Fail { message }) => {
                    let duration_ms = start_time.elapsed().as_millis();
                    self.event_emitter.emit(SessionEvent::error(
                        self.id.clone(),
                        format!("tool pre-hook failed '{}': {}", tool_call.name, message),
                    ))?;
                    self.event_emitter.emit(SessionEvent::tool_call_end(
                        self.id.clone(),
                        tool_call.id.clone(),
                        Option::<String>::None,
                        Some(message.clone()),
                        duration_ms,
                        true,
                    ))?;
                    return Ok(ToolResult {
                        tool_call_id: tool_call.id,
                        content: Value::String(message),
                        is_error: true,
                    });
                }
                Err(error) => {
                    if self.config.tool_hook_strict {
                        let message =
                            format!("tool pre-hook error for '{}': {}", tool_call.name, error);
                        let duration_ms = start_time.elapsed().as_millis();
                        self.event_emitter
                            .emit(SessionEvent::error(self.id.clone(), message.clone()))?;
                        self.event_emitter.emit(SessionEvent::tool_call_end(
                            self.id.clone(),
                            tool_call.id.clone(),
                            Option::<String>::None,
                            Some(message.clone()),
                            duration_ms,
                            true,
                        ))?;
                        return Ok(ToolResult {
                            tool_call_id: tool_call.id,
                            content: Value::String(message),
                            is_error: true,
                        });
                    }
                    self.event_emitter.emit(SessionEvent::warning(
                        self.id.clone(),
                        format!(
                            "tool pre-hook error for '{}': {}; continuing",
                            tool_call.name, error
                        ),
                    ))?;
                }
            }
        }

        let output = match tool_call.name.as_str() {
            "spawn_agent" => self.handle_spawn_agent(arguments).await,
            "send_input" => self.handle_send_input(arguments).await,
            "wait" => self.handle_wait(arguments).await,
            "close_agent" => self.handle_close_agent(arguments).await,
            _ => Err(ToolError::UnknownTool(tool_call.name.clone()).into()),
        };

        match output {
            Ok(raw_output) => {
                let duration_ms = start_time.elapsed().as_millis();
                self.event_emitter.emit(SessionEvent::tool_call_end(
                    self.id.clone(),
                    tool_call.id.clone(),
                    Some(raw_output.clone()),
                    Option::<String>::None,
                    duration_ms,
                    false,
                ))?;
                if let Some(hook) = &self.tool_call_hook {
                    let hook_context = crate::ToolPostHookContext {
                        tool: crate::ToolHookContext {
                            session_id: self.id.clone(),
                            call_id: tool_call.id.clone(),
                            tool_name: tool_call.name.clone(),
                            arguments: parse_tool_call_arguments(&tool_call)?,
                        },
                        duration_ms,
                        output: Some(raw_output.clone()),
                        error: None,
                        is_error: false,
                    };
                    if let Err(error) = hook.after_tool_call(&hook_context).await {
                        if self.config.tool_hook_strict {
                            return Ok(ToolResult {
                                tool_call_id: tool_call.id,
                                content: Value::String(format!(
                                    "tool post-hook error for '{}': {}",
                                    tool_call.name, error
                                )),
                                is_error: true,
                            });
                        }
                        self.event_emitter.emit(SessionEvent::warning(
                            self.id.clone(),
                            format!(
                                "tool post-hook error for '{}': {}; continuing",
                                tool_call.name, error
                            ),
                        ))?;
                    }
                }
                let truncated = truncate_tool_output(&raw_output, &tool_call.name, &self.config);
                Ok(ToolResult {
                    tool_call_id: tool_call.id,
                    content: Value::String(truncated),
                    is_error: false,
                })
            }
            Err(error) => {
                let message = error.to_string();
                let duration_ms = start_time.elapsed().as_millis();
                self.event_emitter.emit(SessionEvent::tool_call_end(
                    self.id.clone(),
                    tool_call.id.clone(),
                    Option::<String>::None,
                    Some(message.clone()),
                    duration_ms,
                    true,
                ))?;
                if let Some(hook) = &self.tool_call_hook {
                    let hook_context = crate::ToolPostHookContext {
                        tool: crate::ToolHookContext {
                            session_id: self.id.clone(),
                            call_id: tool_call.id.clone(),
                            tool_name: tool_call.name.clone(),
                            arguments: parse_tool_call_arguments(&tool_call)?,
                        },
                        duration_ms,
                        output: None,
                        error: Some(message.clone()),
                        is_error: true,
                    };
                    if let Err(error) = hook.after_tool_call(&hook_context).await {
                        if self.config.tool_hook_strict {
                            return Ok(ToolResult {
                                tool_call_id: tool_call.id,
                                content: Value::String(format!(
                                    "tool post-hook error for '{}': {}",
                                    tool_call.name, error
                                )),
                                is_error: true,
                            });
                        }
                        self.event_emitter.emit(SessionEvent::warning(
                            self.id.clone(),
                            format!(
                                "tool post-hook error for '{}': {}; continuing",
                                tool_call.name, error
                            ),
                        ))?;
                    }
                }
                Ok(ToolResult {
                    tool_call_id: tool_call.id,
                    content: Value::String(message),
                    is_error: true,
                })
            }
        }
    }

    pub(super) async fn handle_spawn_agent(
        &mut self,
        arguments: Value,
    ) -> Result<String, AgentError> {
        if self.subagent_depth >= self.config.max_subagent_depth {
            return Err(ToolError::Execution(format!(
                "max_subagent_depth={} reached; recursive spawning is blocked",
                self.config.max_subagent_depth
            ))
            .into());
        }

        let task = required_string_argument(&arguments, "task")?;
        let working_dir = optional_string_argument(&arguments, "working_dir")?;
        let model_override = optional_string_argument(&arguments, "model")?;
        let requested_max_turns = optional_usize_argument(&arguments, "max_turns")?;
        let mut child_config = self.config.clone();
        child_config.max_turns = requested_max_turns.unwrap_or(50);
        child_config.max_subagent_depth = self.config.max_subagent_depth;

        let child_execution_env: Arc<dyn ExecutionEnvironment> =
            if let Some(working_dir) = working_dir {
                let scoped_dir = resolve_subagent_working_directory(
                    self.execution_env.working_directory(),
                    &working_dir,
                )?;
                Arc::new(ScopedExecutionEnvironment::new(
                    self.execution_env.clone(),
                    scoped_dir,
                ))
            } else {
                self.execution_env.clone()
            };

        let child_provider_profile: Arc<dyn ProviderProfile> =
            if let Some(model) = model_override.filter(|value| !value.trim().is_empty()) {
                Arc::new(ModelOverrideProviderProfile::new(
                    self.provider_profile.clone(),
                    model,
                ))
            } else {
                self.provider_profile.clone()
            };

        let child_id = Uuid::new_v4().to_string();
        self.subagents.insert(
            child_id.clone(),
            SubAgentHandle {
                id: child_id.clone(),
                status: SubAgentStatus::Running,
            },
        );

        let mut child_session = Session::new_with_depth(
            child_provider_profile,
            child_execution_env,
            self.llm_client.clone(),
            child_config,
            self.event_emitter.clone(),
            self.persistence_writer.clone(),
            self.subagent_depth + 1,
        )?;

        let mut parent_turn_id: Option<String> = None;
        if self.persistence_enabled() {
            self.ensure_persistence_context().await?;
            if let (Some(store), Some(context_id)) = (
                self.persistence_writer.clone(),
                self.persistence_context_id.clone(),
            ) {
                match store.get_head(&context_id).await {
                    Ok(head) => parent_turn_id = Some(head.turn_id),
                    Err(error) => self.handle_persistence_error(error, "get_head")?,
                }
            }
        }

        if child_session.persistence_enabled() && child_session.persistence_context_id.is_none() {
            if let Some(store) = child_session.persistence_writer.clone() {
                let base_turn = parent_turn_id
                    .as_ref()
                    .filter(|turn_id| turn_id.as_str() != "0")
                    .cloned();
                match store.create_context(base_turn).await {
                    Ok(context) => {
                        child_session.persistence_parent_turn_id = if context.head_turn_id == "0" {
                            None
                        } else {
                            Some(context.head_turn_id.clone())
                        };
                        child_session.persistence_context_id = Some(context.context_id);
                    }
                    Err(error) => {
                        child_session.handle_persistence_error(error, "create_context")?
                    }
                }
            }
        }

        let child_context_id = child_session.persistence_context_id.clone();
        let session_id = self.id.clone();
        let thread_key = self.thread_key.clone();
        let child_session_id = child_session.id.clone();
        let subagent_id = child_id.clone();
        self.persist_typed_payload(
            "forge.link.subagent_spawn",
            "subagent_spawn",
            AgentTurnRecord {
                session_id: self.id.clone(),
                timestamp: current_timestamp(),
                turn: serde_json::json!({
                    "session_id": session_id,
                    "parent_turn": parent_turn_id,
                    "child_context_id": child_context_id,
                    "thread_key": thread_key,
                    "subagent_id": subagent_id,
                    "child_session_id": child_session_id,
                }),
                sequence_no: 0,
                thread_key: self.thread_key.clone(),
                fs_root_hash: None,
                snapshot_policy_id: None,
                snapshot_stats: None,
            },
        )
        .await?;

        let active_task = Some(spawn_subagent_submit_task(Box::new(child_session), task));
        self.subagent_records.insert(
            child_id.clone(),
            SubAgentRecord {
                session: None,
                active_task,
                result: None,
            },
        );
        tokio::task::yield_now().await;

        Ok(serde_json::json!({
            "agent_id": child_id,
            "status": subagent_status_label(&SubAgentStatus::Running),
        })
        .to_string())
    }

    pub(super) async fn handle_send_input(
        &mut self,
        arguments: Value,
    ) -> Result<String, AgentError> {
        let agent_id = required_string_argument(&arguments, "agent_id")?;
        let message = required_string_argument(&arguments, "message")?;
        let mut record = self
            .subagent_records
            .remove(&agent_id)
            .ok_or_else(|| ToolError::Execution(format!("subagent '{}' not found", agent_id)))?;
        self.reconcile_subagent_record(&agent_id, &mut record, false)
            .await?;

        if record.active_task.is_some() {
            self.subagent_records.insert(agent_id.clone(), record);
            return Err(ToolError::Execution(format!(
                "subagent '{}' is still running; call wait before send_input",
                agent_id
            ))
            .into());
        }

        let Some(session) = record.session.take() else {
            self.subagent_records.insert(agent_id.clone(), record);
            return Err(ToolError::Execution(format!(
                "subagent '{}' is unavailable for new input",
                agent_id
            ))
            .into());
        };

        record.active_task = Some(spawn_subagent_submit_task(session, message));
        self.set_subagent_status(&agent_id, SubAgentStatus::Running);
        self.subagent_records.insert(agent_id.clone(), record);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": subagent_status_label(&SubAgentStatus::Running),
        })
        .to_string())
    }

    pub(super) async fn handle_wait(&mut self, arguments: Value) -> Result<String, AgentError> {
        let agent_id = required_string_argument(&arguments, "agent_id")?;
        let mut record = self
            .subagent_records
            .remove(&agent_id)
            .ok_or_else(|| ToolError::Execution(format!("subagent '{}' not found", agent_id)))?;
        self.reconcile_subagent_record(&agent_id, &mut record, true)
            .await?;

        let result = record.result.clone().unwrap_or(SubAgentResult {
            output: String::new(),
            success: matches!(
                self.subagents.get(&agent_id).map(|handle| &handle.status),
                Some(SubAgentStatus::Completed)
            ),
            turns_used: record
                .session
                .as_ref()
                .map(|session| session.history().len())
                .unwrap_or_default(),
        });
        self.subagent_records.insert(agent_id.clone(), record);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": subagent_status_label(self.subagents.get(&agent_id).map(|h| &h.status).unwrap_or(&SubAgentStatus::Failed)),
            "output": result.output,
            "success": result.success,
            "turns_used": result.turns_used
        })
        .to_string())
    }

    pub(super) async fn handle_close_agent(
        &mut self,
        arguments: Value,
    ) -> Result<String, AgentError> {
        let agent_id = required_string_argument(&arguments, "agent_id")?;
        let mut record = self
            .subagent_records
            .remove(&agent_id)
            .ok_or_else(|| ToolError::Execution(format!("subagent '{}' not found", agent_id)))?;
        if let Some(task) = record.active_task.take() {
            task.abort();
        }
        if let Some(session) = record.session.as_mut() {
            session.request_abort();
            let _ = session.close();
        }
        self.set_subagent_status(&agent_id, SubAgentStatus::Failed);
        self.subagent_records.insert(agent_id.clone(), record);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": "closed"
        })
        .to_string())
    }

    pub(super) async fn reconcile_subagent_record(
        &mut self,
        agent_id: &str,
        record: &mut SubAgentRecord,
        wait_for_completion: bool,
    ) -> Result<(), AgentError> {
        let Some(task) = record.active_task.take() else {
            return Ok(());
        };

        if !wait_for_completion && !task.is_finished() {
            record.active_task = Some(task);
            self.set_subagent_status(agent_id, SubAgentStatus::Running);
            return Ok(());
        }

        match task.await {
            Ok(output) => {
                let status = if output.result.success {
                    SubAgentStatus::Completed
                } else {
                    SubAgentStatus::Failed
                };
                record.session = Some(output.session);
                record.result = Some(output.result);
                self.set_subagent_status(agent_id, status);
            }
            Err(error) => {
                record.result = Some(SubAgentResult {
                    output: format!("subagent task join failed: {}", error),
                    success: false,
                    turns_used: 0,
                });
                self.set_subagent_status(agent_id, SubAgentStatus::Failed);
            }
        }

        Ok(())
    }

    pub(super) fn set_subagent_status(&mut self, agent_id: &str, status: SubAgentStatus) {
        if let Some(handle) = self.subagents.get_mut(agent_id) {
            handle.status = status;
        }
    }
    pub(super) fn close_all_subagents(&mut self) -> Result<(), AgentError> {
        let agent_ids: Vec<String> = self.subagent_records.keys().cloned().collect();
        for agent_id in agent_ids {
            if let Some(record) = self.subagent_records.get_mut(&agent_id) {
                if let Some(task) = record.active_task.take() {
                    task.abort();
                }
                if let Some(session) = record.session.as_mut() {
                    let _ = session.close();
                }
            }
            self.set_subagent_status(&agent_id, SubAgentStatus::Failed);
        }
        Ok(())
    }
}
