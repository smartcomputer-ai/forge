//! Typed data-plane client methods.

use environment_protocol::data::methods::{FS_CAPTURE_METHOD, FS_MATERIALIZE_METHOD};
use environment_protocol::data::transfer::*;
use environment_protocol::data::{
    fs::{
        CopyParams, CopyResponse, CreateDirectoryParams, CreateDirectoryResponse,
        GetMetadataParams, GetMetadataResponse, GlobFilesParams, GlobFilesResponse,
        ReadDirectoryParams, ReadDirectoryResponse, ReadFileParams, ReadFileResponse, RemoveParams,
        RemoveResponse, SearchTextParams, SearchTextResponse, WriteFileParams, WriteFileResponse,
    },
    handshake::{InitializeParams, InitializeResponse, InitializedParams},
    idle::{IdleParams, IdleResponse},
    jobs::{
        CancelJobsParams, CancelJobsResponse, ListJobsParams, ListJobsResponse, ReadJobsParams,
        ReadJobsResponse, StartJobsParams, StartJobsResponse,
    },
    methods::{
        ENV_IDLE_METHOD, FS_COPY_METHOD, FS_CREATE_DIRECTORY_METHOD, FS_GET_METADATA_METHOD,
        FS_GLOB_FILES_METHOD, FS_READ_DIRECTORY_METHOD, FS_READ_FILE_METHOD, FS_REMOVE_METHOD,
        FS_SEARCH_TEXT_METHOD, FS_WRITE_FILE_METHOD, INITIALIZE_METHOD, INITIALIZED_METHOD,
        JOB_CANCEL_METHOD, JOB_LIST_METHOD, JOB_READ_METHOD, JOB_START_METHOD, PROCESS_READ_METHOD,
        PROCESS_RESIZE_METHOD, PROCESS_START_METHOD, PROCESS_TERMINATE_METHOD,
        PROCESS_WRITE_METHOD,
    },
    process::{
        ReadProcessParams, ReadProcessResponse, ResizeProcessParams, ResizeProcessResponse,
        StartProcessParams, StartProcessResponse, TerminateProcessParams, TerminateProcessResponse,
        WriteProcessParams, WriteProcessResponse,
    },
};

use crate::{
    error::EnvironmentClientResult,
    rpc::{JsonRpcClient, JsonRpcNotification, JsonRpcTransport},
    transport::{WebSocketConnectOptions, WebSocketTransport},
};

pub struct EnvironmentDataClient<T> {
    rpc: JsonRpcClient<T>,
}

impl<T> EnvironmentDataClient<T>
where
    T: JsonRpcTransport,
{
    pub fn new(transport: T) -> Self {
        Self {
            rpc: JsonRpcClient::new(transport),
        }
    }

    pub fn from_rpc(rpc: JsonRpcClient<T>) -> Self {
        Self { rpc }
    }

    pub fn into_rpc(self) -> JsonRpcClient<T> {
        self.rpc
    }

    /// Gracefully close the underlying transport.
    pub async fn close(&mut self) -> EnvironmentClientResult<()> {
        self.rpc.close().await
    }

    pub async fn initialize(
        &mut self,
        params: &InitializeParams,
    ) -> EnvironmentClientResult<InitializeResponse> {
        self.rpc.request(INITIALIZE_METHOD, params).await
    }

    pub async fn initialized(&mut self, params: &InitializedParams) -> EnvironmentClientResult<()> {
        self.rpc.notify(INITIALIZED_METHOD, params).await
    }

    pub async fn capture(
        &mut self,
        params: &CaptureParams,
    ) -> EnvironmentClientResult<CaptureResponse> {
        self.rpc.request(FS_CAPTURE_METHOD, params).await
    }

    pub async fn materialize(
        &mut self,
        params: &MaterializeParams,
    ) -> EnvironmentClientResult<MaterializeResponse> {
        self.rpc.request(FS_MATERIALIZE_METHOD, params).await
    }

    pub async fn read_file(
        &mut self,
        params: &ReadFileParams,
    ) -> EnvironmentClientResult<ReadFileResponse> {
        self.rpc.request(FS_READ_FILE_METHOD, params).await
    }

    pub async fn write_file(
        &mut self,
        params: &WriteFileParams,
    ) -> EnvironmentClientResult<WriteFileResponse> {
        self.rpc.request(FS_WRITE_FILE_METHOD, params).await
    }

    pub async fn create_directory(
        &mut self,
        params: &CreateDirectoryParams,
    ) -> EnvironmentClientResult<CreateDirectoryResponse> {
        self.rpc.request(FS_CREATE_DIRECTORY_METHOD, params).await
    }

    pub async fn get_metadata(
        &mut self,
        params: &GetMetadataParams,
    ) -> EnvironmentClientResult<GetMetadataResponse> {
        self.rpc.request(FS_GET_METADATA_METHOD, params).await
    }

    pub async fn read_directory(
        &mut self,
        params: &ReadDirectoryParams,
    ) -> EnvironmentClientResult<ReadDirectoryResponse> {
        self.rpc.request(FS_READ_DIRECTORY_METHOD, params).await
    }

    pub async fn remove(
        &mut self,
        params: &RemoveParams,
    ) -> EnvironmentClientResult<RemoveResponse> {
        self.rpc.request(FS_REMOVE_METHOD, params).await
    }

    pub async fn copy(&mut self, params: &CopyParams) -> EnvironmentClientResult<CopyResponse> {
        self.rpc.request(FS_COPY_METHOD, params).await
    }

    pub async fn search_text(
        &mut self,
        params: &SearchTextParams,
    ) -> EnvironmentClientResult<SearchTextResponse> {
        self.rpc.request(FS_SEARCH_TEXT_METHOD, params).await
    }

    pub async fn glob_files(
        &mut self,
        params: &GlobFilesParams,
    ) -> EnvironmentClientResult<GlobFilesResponse> {
        self.rpc.request(FS_GLOB_FILES_METHOD, params).await
    }

    pub async fn start_process(
        &mut self,
        params: &StartProcessParams,
    ) -> EnvironmentClientResult<StartProcessResponse> {
        self.rpc.request(PROCESS_START_METHOD, params).await
    }

    pub async fn read_process(
        &mut self,
        params: &ReadProcessParams,
    ) -> EnvironmentClientResult<ReadProcessResponse> {
        self.rpc.request(PROCESS_READ_METHOD, params).await
    }

    pub async fn write_process(
        &mut self,
        params: &WriteProcessParams,
    ) -> EnvironmentClientResult<WriteProcessResponse> {
        self.rpc.request(PROCESS_WRITE_METHOD, params).await
    }

    pub async fn terminate_process(
        &mut self,
        params: &TerminateProcessParams,
    ) -> EnvironmentClientResult<TerminateProcessResponse> {
        self.rpc.request(PROCESS_TERMINATE_METHOD, params).await
    }

    pub async fn resize_process(
        &mut self,
        params: &ResizeProcessParams,
    ) -> EnvironmentClientResult<ResizeProcessResponse> {
        self.rpc.request(PROCESS_RESIZE_METHOD, params).await
    }

    pub async fn start_jobs(
        &mut self,
        params: &StartJobsParams,
    ) -> EnvironmentClientResult<StartJobsResponse> {
        self.rpc.request(JOB_START_METHOD, params).await
    }

    pub async fn list_jobs(
        &mut self,
        params: &ListJobsParams,
    ) -> EnvironmentClientResult<ListJobsResponse> {
        self.rpc.request(JOB_LIST_METHOD, params).await
    }

    pub async fn read_jobs(
        &mut self,
        params: &ReadJobsParams,
    ) -> EnvironmentClientResult<ReadJobsResponse> {
        self.rpc.request(JOB_READ_METHOD, params).await
    }

    pub async fn cancel_jobs(
        &mut self,
        params: &CancelJobsParams,
    ) -> EnvironmentClientResult<CancelJobsResponse> {
        self.rpc.request(JOB_CANCEL_METHOD, params).await
    }

    /// Daemon activity report for idle power policy. Not itself activity.
    pub async fn idle(&mut self, params: &IdleParams) -> EnvironmentClientResult<IdleResponse> {
        self.rpc.request(ENV_IDLE_METHOD, params).await
    }

    pub async fn next_notification(
        &mut self,
    ) -> EnvironmentClientResult<Option<JsonRpcNotification>> {
        self.rpc.next_notification().await
    }
}

impl EnvironmentDataClient<WebSocketTransport> {
    pub async fn connect(
        endpoint: &str,
        options: WebSocketConnectOptions,
    ) -> EnvironmentClientResult<Self> {
        Ok(Self::new(
            WebSocketTransport::connect(endpoint, options).await?,
        ))
    }
}
