use std::collections::{BTreeMap, BTreeSet};

use api_projection::{MAX_EVENT_PAGE_LIMIT, read_all_session_entries, replay_core_agent_state};
use engine::{BlobRef, PromiseSourceCheckResult, storage::BlobStore};
use environments::{
    EnvironmentId, EnvironmentInstanceId, EnvironmentInstanceStore, EnvironmentJobGroupId,
    SessionEnvironmentBindingState, SessionEnvironmentBindingStore,
};
use host_client::{HostClientError, HostDataClient, WebSocketConnectOptions};
use host_protocol::{
    control::targets::HostTargetStatus,
    data::{
        handshake::{InitializeParams, InitializedParams},
        jobs::{CancelJobsParams, JobStatus, ReadJobsParams},
    },
    error::HostErrorCode,
    shared::{CURRENT_PROTOCOL_VERSION, HostConnectionSpec, HostTransport},
};
use store_pg::PgStore;
use temporal_workflow::{
    EnvironmentJobCancelActivityRequest, EnvironmentJobPollActivityRequest,
    EnvironmentJobPollActivityResult, EnvironmentJobPrepareWorkflowToolRequest,
    EnvironmentJobStartActivityRequest, EnvironmentJobStartActivityResult,
    EnvironmentJobStartPayload, EnvironmentJobSubscription, EnvironmentJobWorkflowArgs,
    EnvironmentJobWorkflowToolContext,
};
use temporalio_common::error::ApplicationFailure;
use temporalio_sdk::activities::ActivityError;

use super::common::activity_error;
use crate::credential_injection::EnvironmentCredentialResolver;

const PROMISE_JOB_OUTPUT_BYTES: usize = 16 * 1024;

pub(super) async fn prepare_workflow_tool(
    store: Option<&std::sync::Arc<PgStore>>,
    request: EnvironmentJobPrepareWorkflowToolRequest,
) -> Result<EnvironmentJobWorkflowArgs, ActivityError> {
    let store = store.ok_or_else(|| {
        activity_error(anyhow::anyhow!(
            "environment job activities are not configured"
        ))
    })?;
    let start = request.start;
    let expected_holder =
        temporal_workflow::compose_workflow_id(start.universe_id, &start.invocation.session_id);
    if start.execution_id.is_empty()
        || start.universe_id != start.invocation.session_universe_id
        || start.holder_workflow_id != expected_holder
    {
        return Err(activity_error(anyhow::anyhow!(
            "environment job workflow-tool start identity is invalid"
        )));
    }
    let arguments = store
        .read_bytes(&start.invocation.arguments_ref)
        .await
        .map_err(activity_error)?;
    let args: tools::environment::jobs::JobStartArgs =
        serde_json::from_slice(&arguments).map_err(activity_error)?;
    if args.jobs.is_empty() {
        return Err(activity_error(anyhow::anyhow!(
            "job_start requires at least one job"
        )));
    }
    let env_id = match args.env_id {
        Some(env_id) => EnvironmentId::try_new(env_id).map_err(activity_error)?,
        None => {
            let entries = read_all_session_entries(
                store.as_ref(),
                &start.invocation.session_id,
                MAX_EVENT_PAGE_LIMIT as usize,
            )
            .await
            .map_err(activity_error)?;
            let state = replay_core_agent_state(&entries).map_err(activity_error)?;
            let target = state
                .tooling
                .routing
                .default_targets
                .get(tools::targets::ENV_TARGET_NAMESPACE)
                .ok_or_else(|| {
                    activity_error(anyhow::anyhow!(
                        "job_start requires env_id or an active environment target"
                    ))
                })?;
            EnvironmentId::try_new(target.id.as_str()).map_err(activity_error)?
        }
    };
    let binding = store
        .read_binding(&start.invocation.session_id, &env_id)
        .await
        .map_err(activity_error)?;
    if binding.state != SessionEnvironmentBindingState::Attached {
        return Err(activity_error(anyhow::anyhow!(
            "environment is detached: {env_id}"
        )));
    }
    let instance = store
        .read_instance(&binding.instance_id)
        .await
        .map_err(activity_error)?;
    if !instance.capabilities.job_start {
        return Err(activity_error(anyhow::anyhow!(
            "environment does not support durable jobs: {env_id}"
        )));
    }

    let request_id = format!(
        "jobreq:{}:{}:{}:{}",
        start.invocation.run_id.as_u64(),
        start.invocation.turn_id.as_u64(),
        start.invocation.tool_batch_id.as_u64(),
        start.invocation.tool_call_id.as_str()
    );
    let jobs = args
        .jobs
        .into_iter()
        .map(|spec| {
            let job_id = spec.job_id.clone();
            spec.into_host_spec(job_id)
                .map_err(|error| activity_error(anyhow::anyhow!(error.to_string())))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let params = host_protocol::data::jobs::StartJobsParams {
        namespace: binding.instance_id.as_str().to_owned(),
        request_id,
        jobs,
    };
    let request_fingerprint =
        BlobRef::from_bytes(&serde_json::to_vec(&params).map_err(activity_error)?);
    let job_group_id = derived_workflow_tool_job_group_id(
        &binding.instance_id,
        &params.request_id,
        request_fingerprint.as_str(),
    );
    let completion_promises = start
        .invocation
        .completion_promises
        .as_ref()
        .ok_or_else(|| {
            activity_error(anyhow::anyhow!(
                "job_start workflow invocation is missing completion promises"
            ))
        })?;
    if completion_promises.len() != params.jobs.len() {
        return Err(activity_error(anyhow::anyhow!(
            "job_start completion promise count does not match job count"
        )));
    }
    let subscriptions = params
        .jobs
        .iter()
        .enumerate()
        .map(|(index, job)| {
            let completion_key = format!("job-{index}");
            let promise_id = completion_promises.get(&completion_key).ok_or_else(|| {
                activity_error(anyhow::anyhow!(
                    "job_start is missing completion key {completion_key}"
                ))
            })?;
            Ok(EnvironmentJobSubscription {
                holder_workflow_id: start.holder_workflow_id.clone(),
                promise_id: promise_id.as_str().to_owned(),
                completion_key,
                job_id: job.job_id.clone(),
                notified: false,
            })
        })
        .collect::<Result<Vec<_>, ActivityError>>()?;
    let job_ids = params.jobs.iter().map(|job| job.job_id.clone()).collect();
    let payload = EnvironmentJobStartPayload {
        request: params,
        credential_scope: Some(temporal_workflow::EnvironmentJobCredentialScope {
            session_id: start.invocation.session_id.clone(),
            env_id: env_id.as_str().to_owned(),
        }),
    };
    let request_ref = store
        .put_bytes(serde_json::to_vec(&payload).map_err(activity_error)?)
        .await
        .map_err(activity_error)?;
    Ok(EnvironmentJobWorkflowArgs {
        universe_id: start.universe_id,
        start: EnvironmentJobStartActivityRequest {
            universe_id: start.universe_id,
            instance_id: binding.instance_id.as_str().to_owned(),
            job_group_id: job_group_id.as_str().to_owned(),
            request_ref,
        },
        job_ids,
        subscriptions,
        started: false,
        jobs: Vec::new(),
        resolutions: BTreeMap::new(),
        poll_ms: 2_000,
        poll_attempt: 0,
        workflow_tool: Some(EnvironmentJobWorkflowToolContext {
            execution_id: start.execution_id,
            invocation_id: start.invocation.invocation_id,
        }),
    })
}

fn derived_workflow_tool_job_group_id(
    instance_id: &EnvironmentInstanceId,
    request_id: &str,
    request_fingerprint: &str,
) -> EnvironmentJobGroupId {
    let hash =
        BlobRef::from_bytes(format!("{instance_id}:{request_id}:{request_fingerprint}").as_bytes());
    EnvironmentJobGroupId::new(format!("ejg_{}", &hash.as_str()[7..31]))
}

pub(super) async fn start(
    store: Option<&std::sync::Arc<PgStore>>,
    request: EnvironmentJobStartActivityRequest,
) -> Result<EnvironmentJobStartActivityResult, ActivityError> {
    let store = store.ok_or_else(|| {
        activity_error(anyhow::anyhow!(
            "environment job activities are not configured"
        ))
    })?;
    let instance_id =
        EnvironmentInstanceId::try_new(request.instance_id.clone()).map_err(activity_error)?;
    let mut payload: EnvironmentJobStartPayload = serde_json::from_slice(
        &store
            .read_bytes(&request.request_ref)
            .await
            .map_err(activity_error)?,
    )
    .map_err(activity_error)?;
    if payload.request.namespace != instance_id.as_str() {
        return Err(activity_error(anyhow::anyhow!(
            "environment job start namespace does not match instance {instance_id}"
        )));
    }

    if let Some(scope) = payload.credential_scope.take() {
        let env_id = EnvironmentId::try_new(scope.env_id).map_err(activity_error)?;
        let binding = store
            .read_binding(&scope.session_id, &env_id)
            .await
            .map_err(activity_error)?;
        if binding.state != SessionEnvironmentBindingState::Attached
            || binding.instance_id != instance_id
        {
            return Err(activity_error(anyhow::anyhow!(
                "environment job credential scope no longer refers to attached instance {instance_id}"
            )));
        }
        let resolver = EnvironmentCredentialResolver::from_pg_store(store.clone());
        for job in &mut payload.request.jobs {
            let secret_env = resolver
                .resolve_secret_env(&scope.session_id, &env_id, &job.env)
                .await
                .map_err(activity_error)?;
            job.secret_env.extend(secret_env);
        }
    }

    start_on_provider(store.as_ref(), &instance_id, &payload).await
}

async fn start_on_provider(
    store: &PgStore,
    instance_id: &EnvironmentInstanceId,
    payload: &EnvironmentJobStartPayload,
) -> Result<EnvironmentJobStartActivityResult, ActivityError> {
    let instance = store
        .read_instance(instance_id)
        .await
        .map_err(activity_error)?;
    if matches!(
        instance.status,
        HostTargetStatus::Closing | HostTargetStatus::Closed
    ) {
        return Err(non_retryable_activity_error(anyhow::anyhow!(
            "cannot start jobs on closing environment instance {instance_id}"
        )));
    }
    let (mut client, capabilities) = initialized_client(&instance.connection).await?;
    if !capabilities.job_start {
        return Err(non_retryable_activity_error(anyhow::anyhow!(
            "environment does not support durable job start: {instance_id}"
        )));
    }
    let response = client
        .start_jobs(&payload.request)
        .await
        .map_err(start_host_activity_error)?;
    let requested_ids = payload
        .request
        .jobs
        .iter()
        .map(|job| job.job_id.clone())
        .collect::<BTreeSet<_>>();
    let returned_ids = response
        .jobs
        .iter()
        .map(|job| job.job_id.clone())
        .collect::<BTreeSet<_>>();
    if requested_ids != returned_ids {
        return Err(non_retryable_activity_error(anyhow::anyhow!(
            "environment provider start response job ids do not match the request"
        )));
    }
    Ok(EnvironmentJobStartActivityResult {
        jobs: response.jobs,
    })
}

fn start_host_activity_error(error: HostClientError) -> ActivityError {
    match &error {
        HostClientError::Host(error)
            if matches!(
                error.code,
                HostErrorCode::InvalidRequest
                    | HostErrorCode::Unauthorized
                    | HostErrorCode::Forbidden
                    | HostErrorCode::NotFound
                    | HostErrorCode::Conflict
                    | HostErrorCode::Unsupported
                    | HostErrorCode::CapabilityUnavailable
            ) =>
        {
            non_retryable_activity_error(anyhow::anyhow!(error.message.clone()))
        }
        _ => activity_error(error),
    }
}

fn non_retryable_activity_error(error: impl Into<anyhow::Error>) -> ActivityError {
    ActivityError::application(ApplicationFailure::non_retryable(error.into()))
}

pub(super) async fn poll(
    store: Option<&PgStore>,
    request: EnvironmentJobPollActivityRequest,
) -> Result<EnvironmentJobPollActivityResult, ActivityError> {
    let store = store.ok_or_else(|| {
        activity_error(anyhow::anyhow!(
            "environment job activities are not configured"
        ))
    })?;
    let instance_id =
        EnvironmentInstanceId::try_new(request.instance_id).map_err(activity_error)?;
    let instance = store
        .read_instance(&instance_id)
        .await
        .map_err(activity_error)?;
    let (mut client, _) = initialized_client(&instance.connection).await?;
    let requested_job_ids = request.job_ids.iter().cloned().collect::<BTreeSet<_>>();
    let response = client
        .read_jobs(&ReadJobsParams {
            namespace: instance_id.as_str().to_owned(),
            jobs: request.job_ids,
            after_seq: None,
            max_bytes: Some(PROMISE_JOB_OUTPUT_BYTES),
            include_artifacts: false,
            wait_ms: None,
        })
        .await
        .map_err(activity_error)?;
    let mut jobs = Vec::with_capacity(response.jobs.len());
    let mut resolutions = BTreeMap::new();
    for result in response.jobs {
        let summary = result.summary.clone();
        if summary.status.is_terminal() {
            let resolution = if summary.status == JobStatus::Succeeded {
                let payload_ref = store
                    .put_bytes(serde_json::to_vec(&result).map_err(activity_error)?)
                    .await
                    .map_err(activity_error)?;
                PromiseSourceCheckResult::Resolved {
                    payload_ref: Some(payload_ref),
                }
            } else {
                let message = summary.failure.clone().unwrap_or_else(|| {
                    format!(
                        "environment job {} ended as {:?}",
                        summary.job_id, summary.status
                    )
                });
                let error_ref = store
                    .put_bytes(message.into_bytes())
                    .await
                    .map_err(activity_error)?;
                PromiseSourceCheckResult::Failed {
                    error_ref: Some(error_ref),
                }
            };
            resolutions.insert(summary.job_id.as_str().to_owned(), resolution);
        }
        jobs.push(summary);
    }
    let returned_job_ids = jobs
        .iter()
        .map(|job| job.job_id.clone())
        .collect::<BTreeSet<_>>();
    let terminal = returned_job_ids == requested_job_ids
        && !jobs.is_empty()
        && jobs.iter().all(|job| job.status.is_terminal());
    Ok(EnvironmentJobPollActivityResult {
        jobs,
        resolutions,
        terminal,
    })
}

pub(super) async fn cancel(
    store: Option<&PgStore>,
    request: EnvironmentJobCancelActivityRequest,
) -> Result<Vec<host_protocol::data::jobs::JobSummary>, ActivityError> {
    let store = store.ok_or_else(|| {
        activity_error(anyhow::anyhow!(
            "environment job activities are not configured"
        ))
    })?;
    let instance_id =
        EnvironmentInstanceId::try_new(request.instance_id).map_err(activity_error)?;
    let instance = store
        .read_instance(&instance_id)
        .await
        .map_err(activity_error)?;
    let (mut client, _) = initialized_client(&instance.connection).await?;
    let response = client
        .cancel_jobs(&CancelJobsParams {
            namespace: instance_id.as_str().to_owned(),
            jobs: request.jobs,
            scope: request.scope,
            force: request.force,
        })
        .await
        .map_err(activity_error)?;
    Ok(response.jobs)
}

async fn initialized_client(
    connection: &HostConnectionSpec,
) -> Result<
    (
        HostDataClient<host_client::WebSocketTransport>,
        host_protocol::shared::HostCapabilities,
    ),
    ActivityError,
> {
    if connection.transport != HostTransport::WebSocket {
        return Err(activity_error(anyhow::anyhow!(
            "unsupported environment job transport: {:?}",
            connection.transport
        )));
    }
    let mut client = HostDataClient::connect(
        &connection.endpoint,
        WebSocketConnectOptions {
            user_agent: Some("lightspeed-environment-job-workflow".to_owned()),
            ..WebSocketConnectOptions::default()
        },
    )
    .await
    .map_err(activity_error)?;
    let response = client
        .initialize(&InitializeParams {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            client_name: "lightspeed-environment-job-workflow".to_owned(),
            scope: connection.scope.clone(),
            resume_connection_id: None,
        })
        .await
        .map_err(activity_error)?;
    if response.protocol_version != CURRENT_PROTOCOL_VERSION {
        return Err(activity_error(anyhow::anyhow!(
            "unsupported host protocol version {}",
            response.protocol_version
        )));
    }
    let capabilities = response.capabilities;
    client
        .initialized(&InitializedParams {})
        .await
        .map_err(activity_error)?;
    Ok((client, capabilities))
}
