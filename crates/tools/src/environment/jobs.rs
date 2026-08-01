//! Durable environment job capability boundary.

use std::collections::BTreeMap;

use async_trait::async_trait;
use host_protocol::{
    data::jobs::{
        CancelJobsParams, CancelJobsResponse, JobArtifact, JobCancelScope, JobDependency,
        JobDependencyPolicy, JobOutputChunk, JobStartSpec, JobStatus, JobSummary, ReadJobsParams,
        ReadJobsResponse, StartJobsParams, StartJobsResponse,
    },
    shared::{ByteChunk, HostPath, JobId},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use engine::{
    BlobRef,
    storage::{BlobEdge, BlobGraphStore, BlobStore, BlobStoreError},
};

use crate::fs::FsPath;

pub const JOB_START_TOOL_NAME: &str = "job_start";
pub const JOB_READ_TOOL_NAME: &str = "job_read";
pub const JOB_START_WORKFLOW_TOOL_ID: &str = "environment-job-start";
pub const JOB_START_WORKFLOW_SEMANTIC_TYPE: &str = "lightspeed.environment.job.start.v1";

pub type JobExecResult<T> = Result<T, JobError>;

#[async_trait]
pub trait JobExecutor: Send + Sync {
    async fn start_jobs(&self, request: StartJobsParams) -> JobExecResult<StartJobsResponse>;

    async fn read_jobs(&self, request: ReadJobsParams) -> JobExecResult<ReadJobsResponse>;

    async fn cancel_jobs(&self, request: CancelJobsParams) -> JobExecResult<CancelJobsResponse>;
}

#[derive(Debug, Error)]
pub enum JobError {
    #[error("environment jobs unsupported: {message}")]
    Unsupported { message: String },

    #[error("invalid environment job request: {message}")]
    InvalidRequest { message: String },

    #[error("environment job execution failed: {message}")]
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobHandleArg {
    pub environment_id: String,
    pub job_id: JobId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobHandle {
    pub environment_id: String,
    pub job_id: JobId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobStartArgs {
    pub jobs: Vec<JobStartSpecArgs>,
}

/// Runtime-owned facts pinned when a durable `job_start` call is accepted.
/// The receiving workflow reads this through the generic invocation's opaque
/// execution-context reference; it is not part of the model-facing schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobStartExecutionContextV1 {
    pub version: u32,
    pub environment_id: String,
    pub allowed_provider_ids: Option<Vec<String>>,
}

impl JobStartExecutionContextV1 {
    pub const VERSION: u32 = 1;

    pub fn new(environment_id: String, allowed_provider_ids: Option<Vec<String>>) -> Self {
        Self {
            version: Self::VERSION,
            environment_id,
            allowed_provider_ids,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStartSpecArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub job_id: JobId,
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<FsPath>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<JobDependency>,
    #[serde(default)]
    pub dependency_policy: JobDependencyPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_key: Option<String>,
}

impl JobStartSpecArgs {
    pub fn into_host_spec(self, job_id: JobId) -> JobExecResult<JobStartSpec> {
        if self.argv.is_empty() {
            return Err(JobError::InvalidRequest {
                message: "job argv must not be empty".to_owned(),
            });
        }
        Ok(JobStartSpec {
            job_id,
            name: self.name,
            argv: self.argv,
            cwd: self.cwd.as_ref().map(host_path).transpose()?,
            env: self.env,
            secret_env: BTreeMap::new(),
            stdin: self.stdin.map(|value| ByteChunk::from(value.into_bytes())),
            timeout_ms: self.timeout_ms,
            depends_on: self.depends_on,
            dependency_policy: self.dependency_policy,
            queue_key: self.queue_key,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobReadArgs {
    pub jobs: Vec<JobHandleArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_seq: Option<u64>,
    #[serde(default)]
    pub include_artifacts: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCancelArgs {
    pub jobs: Vec<JobHandleArg>,
    #[serde(default)]
    pub scope: JobCancelScope,
    #[serde(default)]
    pub force: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStartResult {
    pub jobs: Vec<JobStarted>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobStarted {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub job_id: JobId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<JobHandle>,
    pub status: JobStatus,
    pub dependencies: Vec<JobId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_key: Option<String>,
    /// Promise settled when this durable job reaches a terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promise: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelJobResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<JobHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<JobSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<ModelJobOutputSegment>,
    pub output_next_seq: u64,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<JobArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelJobResultSet {
    pub jobs: Vec<ModelJobResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelJobOutputSegment {
    pub stream: host_protocol::data::jobs::JobOutputStream,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob_ref: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_len: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCancelResultSet {
    pub jobs: Vec<JobCancelResultEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCancelResultEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<JobHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<JobSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn is_environment_job_query_tool_name(name: &str) -> bool {
    name == JOB_READ_TOOL_NAME
}

pub fn visible_job_read_output(jobs: &[ModelJobResult]) -> String {
    serde_json::to_string_pretty(&ModelJobResultSet {
        jobs: jobs.to_vec(),
    })
    .unwrap_or_else(|error| format!("failed to encode job results: {error}"))
}

pub async fn normalize_job_result(
    blobs: &dyn BlobStore,
    handle: Option<JobHandle>,
    summary: Option<JobSummary>,
    output_chunks: Vec<JobOutputChunk>,
    output_next_seq: u64,
    artifacts: Vec<JobArtifact>,
    error: Option<String>,
    output_bytes: Option<usize>,
) -> Result<ModelJobResult, BlobStoreError> {
    let observed_bytes = output_chunks
        .iter()
        .map(|chunk| chunk.chunk.as_slice().len())
        .sum::<usize>();
    let truncated = output_bytes.is_some_and(|limit| observed_bytes >= limit);
    let mut output = Vec::new();
    let mut index = 0;
    while index < output_chunks.len() {
        let stream = output_chunks[index].stream;
        let mut bytes = Vec::new();
        while index < output_chunks.len() && output_chunks[index].stream == stream {
            bytes.extend_from_slice(output_chunks[index].chunk.as_slice());
            index += 1;
        }
        match String::from_utf8(bytes) {
            Ok(text) => {
                if !text.is_empty() {
                    if let Some(ModelJobOutputSegment {
                        stream: previous_stream,
                        text: Some(previous),
                        ..
                    }) = output.last_mut()
                        && *previous_stream == stream
                    {
                        previous.push_str(&text);
                    } else {
                        output.push(ModelJobOutputSegment {
                            stream,
                            text: Some(text),
                            blob_ref: None,
                            media_type: None,
                            byte_len: None,
                        });
                    }
                }
            }
            Err(error) => {
                let bytes = error.into_bytes();
                let byte_len = bytes.len();
                let blob_ref = blobs.put_bytes(bytes).await?;
                output.push(ModelJobOutputSegment {
                    stream,
                    text: None,
                    blob_ref: Some(blob_ref),
                    media_type: Some("application/octet-stream".to_owned()),
                    byte_len: Some(byte_len),
                });
            }
        }
    }
    Ok(ModelJobResult {
        handle,
        summary,
        output,
        output_next_seq,
        truncated,
        artifacts,
        error,
    })
}

pub async fn store_model_job_result(
    blobs: &dyn BlobStore,
    blob_graph: Option<&dyn BlobGraphStore>,
    result: &ModelJobResult,
) -> Result<BlobRef, BlobStoreError> {
    let root = blobs
        .put_bytes(
            serde_json::to_vec(result).map_err(|error| BlobStoreError::Store {
                message: error.to_string(),
            })?,
        )
        .await?;
    let edges = result
        .output
        .iter()
        .filter_map(|segment| {
            segment
                .blob_ref
                .clone()
                .map(|child| BlobEdge::contains(root.clone(), child))
        })
        .collect::<Vec<_>>();
    if !edges.is_empty()
        && let Some(blob_graph) = blob_graph
    {
        blob_graph.record_blob_edges(edges).await?;
    }
    Ok(root)
}

fn host_path(path: &FsPath) -> JobExecResult<HostPath> {
    HostPath::new(path.as_str()).map_err(|error| JobError::InvalidRequest {
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use engine::storage::{
        BlobEdge, BlobGraphStore, BlobStore, BlobStoreError, InMemoryBlobStore, SessionBlobRoot,
    };
    use host_protocol::data::jobs::{JobOutputStream, JobStatus};
    use serde_json::json;

    use super::*;

    #[test]
    fn job_start_arguments_do_not_accept_an_environment_override() {
        let error = serde_json::from_value::<JobStartArgs>(json!({
            "environment_id": "environment-other",
            "jobs": []
        }))
        .expect_err("environment override must not be model-facing");

        assert!(error.to_string().contains("unknown field `environment_id`"));
    }

    #[test]
    fn job_start_execution_context_is_versioned_and_runtime_owned() {
        let context = JobStartExecutionContextV1::new(
            "environment-active".to_owned(),
            Some(vec!["provider-a".to_owned()]),
        );

        assert_eq!(context.version, JobStartExecutionContextV1::VERSION);
        assert_eq!(context.environment_id, "environment-active");
        assert_eq!(
            context.allowed_provider_ids,
            Some(vec!["provider-a".to_owned()])
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn normalizer_reassembles_utf8_and_preserves_stream_order() {
        let blobs = InMemoryBlobStore::new();
        let result = normalize_job_result(
            &blobs,
            None,
            Some(test_summary()),
            vec![
                output_chunk(0, JobOutputStream::Stdout, vec![b'a', 0xe2]),
                output_chunk(1, JobOutputStream::Stdout, vec![0x82, 0xac, b'\n']),
                output_chunk(2, JobOutputStream::Stderr, b"warning\n".to_vec()),
                output_chunk(3, JobOutputStream::Stdout, b"done\n".to_vec()),
            ],
            4,
            Vec::new(),
            None,
            Some(1024),
        )
        .await
        .expect("normalize text");

        assert_eq!(result.output.len(), 3);
        assert_eq!(result.output[0].text.as_deref(), Some("a€\n"));
        assert_eq!(result.output[1].stream, JobOutputStream::Stderr);
        assert_eq!(result.output[2].text.as_deref(), Some("done\n"));
        assert!(!result.truncated);
        let value = serde_json::to_value(&result).expect("semantic JSON");
        assert!(value.get("outputChunks").is_none());
        assert_eq!(value["outputNextSeq"], 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn normalizer_stores_binary_output_as_a_ref() {
        let blobs = InMemoryBlobStore::new();
        let result = normalize_job_result(
            &blobs,
            None,
            Some(test_summary()),
            vec![output_chunk(0, JobOutputStream::Stdout, vec![0xff, 0x00])],
            1,
            Vec::new(),
            None,
            Some(2),
        )
        .await
        .expect("normalize binary");

        assert!(result.truncated);
        let segment = &result.output[0];
        assert!(segment.text.is_none());
        assert_eq!(
            segment.media_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(segment.byte_len, Some(2));
        let blob_ref = segment.blob_ref.as_ref().expect("binary ref");
        assert_eq!(
            blobs.read_bytes(blob_ref).await.expect("binary bytes"),
            vec![0xff, 0x00]
        );
        let encoded = serde_json::to_string(&result).expect("semantic JSON");
        assert!(!encoded.contains("/wA="));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stored_job_manifest_records_binary_child_edges() {
        let blobs = InMemoryBlobStore::new();
        let graph = RecordingGraph::default();
        let result = normalize_job_result(
            &blobs,
            None,
            Some(test_summary()),
            vec![output_chunk(0, JobOutputStream::Stdout, vec![0xff])],
            1,
            Vec::new(),
            None,
            None,
        )
        .await
        .expect("normalize binary");
        let child = result.output[0].blob_ref.clone().expect("binary child ref");
        let root = store_model_job_result(&blobs, Some(&graph), &result)
            .await
            .expect("store manifest");

        assert_eq!(
            graph.edges.lock().expect("edge lock").as_slice(),
            &[BlobEdge::contains(root, child)]
        );
    }

    #[derive(Default)]
    struct RecordingGraph {
        edges: Arc<Mutex<Vec<BlobEdge>>>,
    }

    #[async_trait]
    impl BlobGraphStore for RecordingGraph {
        async fn record_session_blob_roots(
            &self,
            _roots: Vec<SessionBlobRoot>,
        ) -> Result<(), BlobStoreError> {
            Ok(())
        }

        async fn record_blob_edges(&self, edges: Vec<BlobEdge>) -> Result<(), BlobStoreError> {
            self.edges.lock().expect("edge lock").extend(edges);
            Ok(())
        }
    }

    fn output_chunk(seq: u64, stream: JobOutputStream, bytes: Vec<u8>) -> JobOutputChunk {
        JobOutputChunk {
            seq,
            stream,
            chunk: ByteChunk::from(bytes),
        }
    }

    fn test_summary() -> JobSummary {
        JobSummary {
            namespace: "environment-a".to_owned(),
            job_id: JobId::new("job-a"),
            name: None,
            status: JobStatus::Succeeded,
            dependencies: Vec::new(),
            created_at_ms: 1,
            queued_at_ms: Some(1),
            started_at_ms: Some(2),
            finished_at_ms: Some(3),
            exit_code: Some(0),
            failure: None,
            queue_key: None,
        }
    }
}
