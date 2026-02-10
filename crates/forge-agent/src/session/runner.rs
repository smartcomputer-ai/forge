use super::*;

impl Session {
    pub(super) async fn drain_steering_queue(&mut self) -> Result<(), AgentError> {
        while let Some(content) = self.pop_steering_message() {
            let turn = Turn::Steering(SteeringTurn::new(content.clone(), current_timestamp()));
            self.push_turn(turn.clone());
            self.persist_turn_if_enabled(&turn).await?;
            self.event_emitter
                .emit(SessionEvent::steering_injected(self.id.clone(), content))?;
        }
        Ok(())
    }

    pub(super) async fn inject_loop_detection_warning_if_needed(
        &mut self,
    ) -> Result<(), AgentError> {
        if !self.config.enable_loop_detection {
            return Ok(());
        }

        if !detect_loop(&self.history, self.config.loop_detection_window) {
            return Ok(());
        }

        let warning = format!(
            "Loop detected: the last {} tool calls follow a repeating pattern. Try a different approach.",
            self.config.loop_detection_window
        );
        if matches!(
            self.history.last(),
            Some(Turn::Steering(turn)) if turn.content == warning
        ) {
            return Ok(());
        }

        let turn = Turn::Steering(SteeringTurn::new(warning.clone(), current_timestamp()));
        self.push_turn(turn.clone());
        self.persist_turn_if_enabled(&turn).await?;
        self.event_emitter
            .emit(SessionEvent::loop_detection(self.id.clone(), warning))?;
        Ok(())
    }

    pub(super) fn emit_context_usage_warning_if_needed(&self) -> Result<bool, AgentError> {
        let context_window_size = self.provider_profile.capabilities().context_window_size;
        if context_window_size == 0 {
            return Ok(false);
        }

        let approx_tokens = approximate_context_tokens(&self.history);
        let warning_threshold = context_window_size.saturating_mul(8) / 10;
        if approx_tokens <= warning_threshold {
            return Ok(false);
        }

        let usage_percent = ((approx_tokens as f64 / context_window_size as f64) * 100.0).round();
        self.event_emitter
            .emit(SessionEvent::context_usage_warning(
                self.id.clone(),
                approx_tokens,
                context_window_size,
                usage_percent as usize,
            ))?;
        Ok(true)
    }

    pub(super) fn build_request(&self, options: &SubmitOptions) -> Result<Request, AgentError> {
        let mut provider_profile = self.resolve_provider_profile(options.provider.as_deref())?;
        if let Some(model_override) = options
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            provider_profile = Arc::new(ModelOverrideProviderProfile::new(
                provider_profile,
                model_override.to_string(),
            ));
        }

        let tools = provider_profile.tools();
        let environment_context = build_environment_context_snapshot(
            provider_profile.as_ref(),
            self.execution_env.as_ref(),
        );
        let project_docs = discover_project_documents(
            self.execution_env.working_directory(),
            provider_profile.as_ref(),
        );
        let system_prompt = provider_profile.build_system_prompt(
            &environment_context,
            &tools,
            &project_docs,
            options
                .system_prompt_override
                .as_deref()
                .or(self.config.system_prompt_override.as_deref()),
        );

        let mut messages = vec![Message::system(system_prompt)];
        messages.extend(convert_history_to_messages(&self.history));

        let tools = if tools.is_empty() { None } else { Some(tools) };
        let tool_choice = tools.as_ref().map(|_| ToolChoice {
            mode: "auto".to_string(),
            tool_name: None,
        });

        if let Some(value) = options.reasoning_effort.as_deref() {
            validate_reasoning_effort(value)?;
        }
        let reasoning_effort = options
            .reasoning_effort
            .as_ref()
            .map(|value| value.to_ascii_lowercase())
            .or_else(|| self.config.reasoning_effort.clone());

        let provider_options = options
            .provider_options
            .clone()
            .or_else(|| provider_profile.provider_options());

        Ok(Request {
            model: provider_profile.model().to_string(),
            messages,
            provider: Some(provider_profile.id().to_string()),
            tools,
            tool_choice,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: None,
            reasoning_effort,
            metadata: options.metadata.clone(),
            provider_options,
        })
    }

    pub(super) fn is_abort_requested(&self) -> bool {
        self.abort_requested.load(Ordering::SeqCst)
    }

    pub(super) fn resolve_provider_profile(
        &self,
        provider_override: Option<&str>,
    ) -> Result<Arc<dyn ProviderProfile>, AgentError> {
        let Some(provider_id) = provider_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(self.provider_profile.clone());
        };
        self.provider_profiles
            .get(provider_id)
            .cloned()
            .ok_or_else(|| {
                SessionError::InvalidConfiguration(format!(
                    "unknown provider override '{}'; register profile before use",
                    provider_id
                ))
                .into()
            })
    }

    pub(super) async fn shutdown_to_closed(&mut self) -> Result<(), AgentError> {
        if self.state == SessionState::Closed {
            return Ok(());
        }

        let _ = self.execution_env.terminate_all_commands().await;
        self.transition_to(SessionState::Closed)
    }
}
