//! Typed controller-plane client methods.

use environment_protocol::control::{
    handshake::{ControllerInitializeParams, ControllerInitializeResponse},
    ingress::{EnsureIngressParams, IngressResponse, RemoveIngressParams},
    methods::{
        ADOPT_TARGET_METHOD, CLOSE_TARGET_METHOD, CREATE_TARGET_METHOD, ENSURE_INGRESS_METHOD,
        GET_TARGET_METHOD, INITIALIZE_METHOD, LIST_TARGETS_METHOD, LIST_TEMPLATES_METHOD,
        REMOVE_INGRESS_METHOD, SET_TARGET_POWER_METHOD,
    },
    targets::{
        AdoptTargetParams, AdoptTargetResponse, CloseTargetParams, CloseTargetResponse,
        CreateTargetParams, CreateTargetResponse, GetTargetParams, GetTargetResponse,
        ListTargetsParams, ListTargetsResponse, ListTemplatesParams, ListTemplatesResponse,
        SetTargetPowerParams, SetTargetPowerResponse,
    },
};

use crate::{
    error::EnvironmentClientResult,
    rpc::{JsonRpcClient, JsonRpcTransport},
    transport::{WebSocketConnectOptions, WebSocketTransport},
};

pub struct EnvironmentProviderClient<T> {
    rpc: JsonRpcClient<T>,
}

impl<T> EnvironmentProviderClient<T>
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
        params: &ControllerInitializeParams,
    ) -> EnvironmentClientResult<ControllerInitializeResponse> {
        self.rpc.request(INITIALIZE_METHOD, params).await
    }

    pub async fn list_targets(
        &mut self,
        params: &ListTargetsParams,
    ) -> EnvironmentClientResult<ListTargetsResponse> {
        self.rpc.request(LIST_TARGETS_METHOD, params).await
    }

    pub async fn list_templates(
        &mut self,
        params: &ListTemplatesParams,
    ) -> EnvironmentClientResult<ListTemplatesResponse> {
        self.rpc.request(LIST_TEMPLATES_METHOD, params).await
    }

    pub async fn create_target(
        &mut self,
        params: &CreateTargetParams,
    ) -> EnvironmentClientResult<CreateTargetResponse> {
        self.rpc.request(CREATE_TARGET_METHOD, params).await
    }

    pub async fn adopt_target(
        &mut self,
        params: &AdoptTargetParams,
    ) -> EnvironmentClientResult<AdoptTargetResponse> {
        self.rpc.request(ADOPT_TARGET_METHOD, params).await
    }

    pub async fn get_target(
        &mut self,
        params: &GetTargetParams,
    ) -> EnvironmentClientResult<GetTargetResponse> {
        self.rpc.request(GET_TARGET_METHOD, params).await
    }

    pub async fn close_target(
        &mut self,
        params: &CloseTargetParams,
    ) -> EnvironmentClientResult<CloseTargetResponse> {
        self.rpc.request(CLOSE_TARGET_METHOD, params).await
    }

    pub async fn set_target_power(
        &mut self,
        params: &SetTargetPowerParams,
    ) -> EnvironmentClientResult<SetTargetPowerResponse> {
        self.rpc.request(SET_TARGET_POWER_METHOD, params).await
    }

    pub async fn ensure_ingress(
        &mut self,
        params: &EnsureIngressParams,
    ) -> EnvironmentClientResult<IngressResponse> {
        self.rpc.request(ENSURE_INGRESS_METHOD, params).await
    }

    pub async fn remove_ingress(
        &mut self,
        params: &RemoveIngressParams,
    ) -> EnvironmentClientResult<IngressResponse> {
        self.rpc.request(REMOVE_INGRESS_METHOD, params).await
    }
}

impl EnvironmentProviderClient<WebSocketTransport> {
    pub async fn connect(
        endpoint: &str,
        options: WebSocketConnectOptions,
    ) -> EnvironmentClientResult<Self> {
        Ok(Self::new(
            WebSocketTransport::connect(endpoint, options).await?,
        ))
    }
}
