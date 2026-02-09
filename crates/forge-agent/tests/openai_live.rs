mod support;

use forge_agent::{ExecutionEnvironment, OpenAiProviderProfile, SessionConfig, SubmitOptions};
use forge_llm::Role;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use support::live::{
    bootstrap_live_session, build_openai_live_client, collect_tool_results,
    find_tool_call_end_output, find_tool_result_with_substring, live_tests_enabled,
    openai_live_model, run_with_retries, submit_with_options_timeout, submit_with_timeout,
};

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires RUN_LIVE_OPENAI_TESTS=1 and OPENAI_API_KEY (env or .env)"]
async fn openai_live_create_then_edit_file_smoke_applies_expected_side_effects() {
    if !live_tests_enabled("RUN_LIVE_OPENAI_TESTS") {
        return;
    }
    let Some((client, _requests)) = build_openai_live_client() else {
        return;
    };
    let model = openai_live_model();
    let profile = Arc::new(OpenAiProviderProfile::with_default_tools(model));

    run_with_retries(|_| {
        let client = client.clone();
        let profile = profile.clone();
        async move {
            let (_workspace, env, _emitter, mut session) =
                bootstrap_live_session(profile, client, SessionConfig::default())?;

            submit_with_timeout(
                &mut session,
                "Use tools to create hello_live.txt with exactly one line: alpha. Reply with DONE when finished.",
            )
            .await?;
            submit_with_timeout(
                &mut session,
                "Use tools to edit hello_live.txt by appending a second line: beta. Reply with DONE when finished.",
            )
            .await?;

            let content = env.read_file("hello_live.txt", None, None).await?;
            assert!(content.contains("alpha"));
            assert!(content.contains("beta"));
            Ok(())
        }
    })
    .await
    .expect("openai live create/edit smoke should pass");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires RUN_LIVE_OPENAI_TESTS=1 and OPENAI_API_KEY (env or .env)"]
async fn openai_live_read_file_truncation_smoke_preserves_full_tool_call_end_output() {
    if !live_tests_enabled("RUN_LIVE_OPENAI_TESTS") {
        return;
    }
    let Some((client, _requests)) = build_openai_live_client() else {
        return;
    };
    let model = openai_live_model();
    let profile = Arc::new(OpenAiProviderProfile::with_default_tools(model));

    run_with_retries(|_| {
        let client = client.clone();
        let profile = profile.clone();
        async move {
            let mut config = SessionConfig::default();
            config
                .tool_output_limits
                .insert("read_file".to_string(), 120);
            let (_workspace, env, emitter, mut session) =
                bootstrap_live_session(profile, client, config)?;

            let large = "0123456789abcdefghijklmnopqrstuvwxyz".repeat(400);
            env.write_file("big.txt", &large).await?;

            submit_with_timeout(
                &mut session,
                "Use read_file to inspect big.txt, then reply with one short sentence.",
            )
            .await?;

            let Some((call_id, truncated, _)) = find_tool_result_with_substring(
                session.history(),
                "[WARNING: Tool output was truncated.",
            ) else {
                panic!("expected truncated tool result");
            };
            let events = emitter.snapshot();
            let full_output = find_tool_call_end_output(&events, &call_id)
                .expect("expected TOOL_CALL_END output for truncated result");
            assert!(full_output.len() > truncated.len());
            assert!(full_output.contains("abcdefghijklmnopqrstuvwxyz"));
            Ok(())
        }
    })
    .await
    .expect("openai live truncation smoke should pass");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires RUN_LIVE_OPENAI_TESTS=1 and OPENAI_API_KEY (env or .env)"]
async fn openai_live_shell_timeout_smoke_returns_timeout_tool_result() {
    if !live_tests_enabled("RUN_LIVE_OPENAI_TESTS") {
        return;
    }
    let Some((client, _requests)) = build_openai_live_client() else {
        return;
    };
    let model = openai_live_model();
    let profile = Arc::new(OpenAiProviderProfile::with_default_tools(model));

    run_with_retries(|_| {
        let client = client.clone();
        let profile = profile.clone();
        async move {
            let mut config = SessionConfig::default();
            config.default_command_timeout_ms = 25;
            config.max_command_timeout_ms = 100;
            let (_workspace, _env, _emitter, mut session) =
                bootstrap_live_session(profile, client, config)?;

            submit_with_timeout(
                &mut session,
                "Run `echo start && sleep 1 && echo done` with the shell tool and report the result.",
            )
            .await?;

            let tool_results = collect_tool_results(session.history());
            assert!(
                tool_results
                    .iter()
                    .any(|(_, text, _)| text.contains("Command timed out")),
                "expected timeout tool result, got: {tool_results:?}"
            );
            Ok(())
        }
    })
    .await
    .expect("openai live shell-timeout smoke should pass");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires RUN_LIVE_OPENAI_TESTS=1 and OPENAI_API_KEY (env or .env)"]
async fn openai_live_submit_with_options_smoke_applies_request_overrides() {
    if !live_tests_enabled("RUN_LIVE_OPENAI_TESTS") {
        return;
    }
    let Some((client, requests)) = build_openai_live_client() else {
        return;
    };
    let model = openai_live_model();
    let profile = Arc::new(OpenAiProviderProfile::with_default_tools(model.clone()));

    run_with_retries(|_| {
        let client = client.clone();
        let requests = requests.clone();
        let profile = profile.clone();
        let model = model.clone();
        async move {
            let (_workspace, _env, _emitter, mut session) =
                bootstrap_live_session(profile, client, SessionConfig::default())?;

            let override_marker = "OPENAI_LIVE_OVERRIDE_MARKER";
            let provider_options = json!({ "openai": {} });
            let mut metadata = HashMap::new();
            metadata.insert("live_case".to_string(), "openai_submit_options".to_string());

            submit_with_options_timeout(
                &mut session,
                "Reply with LIVE_OK.",
                SubmitOptions {
                    provider: Some("openai".to_string()),
                    model: Some(model.clone()),
                    reasoning_effort: Some("low".to_string()),
                    system_prompt_override: Some(override_marker.to_string()),
                    provider_options: Some(provider_options.clone()),
                    metadata: Some(metadata),
                },
            )
            .await?;

            let seen = requests.lock().expect("requests mutex").clone();
            let last = seen.last().expect("expected at least one request");
            assert_eq!(last.provider.as_deref(), Some("openai"));
            assert_eq!(last.model, model);
            assert_eq!(last.reasoning_effort.as_deref(), Some("low"));
            assert_eq!(last.provider_options, Some(provider_options));
            assert_eq!(
                last.metadata
                    .as_ref()
                    .and_then(|map| map.get("live_case").map(String::as_str)),
                Some("openai_submit_options")
            );
            assert_eq!(
                last.messages.first().map(|message| message.role.clone()),
                Some(Role::System)
            );
            assert!(
                last.messages
                    .first()
                    .map(|message| message.text().contains(override_marker))
                    .unwrap_or(false)
            );
            Ok(())
        }
    })
    .await
    .expect("openai live submit_with_options smoke should pass");
}
