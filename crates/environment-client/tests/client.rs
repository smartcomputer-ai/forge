use std::collections::{BTreeMap, VecDeque};

use async_trait::async_trait;
use environment_client::{
    EnvironmentClientError, EnvironmentClientResult, EnvironmentDataClient,
    EnvironmentProviderClient, JsonRpcTransport, WebSocketConnectOptions,
};
use environment_protocol::{
    control::{
        handshake::ControllerInitializeParams,
        targets::{AdoptTargetParams, ProviderBindingContext},
    },
    data::{
        fs::ReadFileParams,
        jobs::{JobDependencyPolicy, JobStartSpec, ListJobsParams, StartJobsParams},
        methods::PROCESS_OUTPUT_METHOD,
    },
    error::EnvironmentProtocolErrorCode,
    shared::{ByteChunk, CURRENT_PROTOCOL_VERSION, EnvironmentPath, JobId},
};
use serde_json::{Value, json};

#[tokio::test(flavor = "current_thread")]
async fn websocket_client_close_sends_a_close_frame() {
    use futures_util::StreamExt as _;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut websocket = accept_async(stream).await.expect("websocket handshake");
        while let Some(message) = websocket.next().await {
            if matches!(message.expect("websocket message"), Message::Close(_)) {
                return true;
            }
        }
        false
    });

    let mut client = EnvironmentDataClient::connect(
        &format!("ws://{address}"),
        WebSocketConnectOptions::default(),
    )
    .await
    .expect("connect client");
    client.close().await.expect("close client");

    let saw_close = tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .expect("server observed close before timeout")
        .expect("server task");
    assert!(saw_close, "client dropped without a WebSocket close frame");
}

#[derive(Default)]
struct MockTransport {
    sent: Vec<Value>,
    recv: VecDeque<Value>,
}

impl MockTransport {
    fn with_recv(messages: impl IntoIterator<Item = Value>) -> Self {
        Self {
            sent: Vec::new(),
            recv: messages.into_iter().collect(),
        }
    }
}

#[async_trait]
impl JsonRpcTransport for MockTransport {
    async fn send(&mut self, message: Value) -> EnvironmentClientResult<()> {
        self.sent.push(message);
        Ok(())
    }

    async fn recv(&mut self) -> EnvironmentClientResult<Option<Value>> {
        Ok(self.recv.pop_front())
    }
}

#[tokio::test]
async fn data_client_sends_typed_request_and_decodes_response() {
    let transport = MockTransport::with_recv([json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "data": "aGk="
        }
    })]);
    let mut client = EnvironmentDataClient::new(transport);

    let response = client
        .read_file(&ReadFileParams {
            path: EnvironmentPath::new("README.md").expect("path"),
            offset: None,
            max_bytes: None,
        })
        .await
        .expect("response");

    assert_eq!(response.data, ByteChunk::from(b"hi".as_slice()));

    let transport = client.into_rpc().into_inner();
    assert_eq!(
        transport.sent,
        vec![json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "fs/readFile",
            "params": {
                "path": "README.md"
            }
        })]
    );
}

#[tokio::test]
async fn data_client_sends_typed_job_start_request() {
    let transport = MockTransport::with_recv([json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "jobs": [
                {
                    "createdAtMs": 1,
                    "jobId": "job-1",
                    "namespace": "session_1",
                    "status": "queued"
                }
            ]
        }
    })]);
    let mut client = EnvironmentDataClient::new(transport);

    let response = client
        .start_jobs(&StartJobsParams {
            namespace: "session_1".to_owned(),
            request_id: "request-1".to_owned(),
            jobs: vec![JobStartSpec {
                job_id: JobId::new("job-1"),
                name: None,
                argv: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "printf ok".to_owned(),
                ],
                cwd: None,
                env: Default::default(),
                secret_env: BTreeMap::new(),
                stdin: None,
                timeout_ms: Some(1_000),
                depends_on: Vec::new(),
                dependency_policy: JobDependencyPolicy::AllSucceeded,
                queue_key: None,
            }],
        })
        .await
        .expect("response");

    assert_eq!(response.jobs[0].job_id.as_str(), "job-1");

    let transport = client.into_rpc().into_inner();
    assert_eq!(transport.sent[0]["method"], "job/start");
    assert_eq!(transport.sent[0]["params"]["jobs"][0]["jobId"], "job-1");
    assert_eq!(transport.sent[0]["params"]["jobs"][0]["argv"][0], "/bin/sh");
}

#[tokio::test]
async fn data_client_sends_typed_job_list_request() {
    let transport = MockTransport::with_recv([json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "jobs": [
                {
                    "createdAtMs": 2,
                    "jobId": "job-2",
                    "namespace": "session_1",
                    "status": "running"
                }
            ]
        }
    })]);
    let mut client = EnvironmentDataClient::new(transport);

    let response = client
        .list_jobs(&ListJobsParams {
            namespace: "session_1".to_owned(),
            limit: Some(5),
        })
        .await
        .expect("response");

    assert_eq!(response.jobs[0].job_id.as_str(), "job-2");

    let transport = client.into_rpc().into_inner();
    assert_eq!(transport.sent[0]["method"], "job/list");
    assert_eq!(transport.sent[0]["params"]["namespace"], "session_1");
    assert_eq!(transport.sent[0]["params"]["limit"], 5);
}

#[tokio::test]
async fn data_client_stashes_notifications_seen_while_waiting_for_response() {
    let transport = MockTransport::with_recv([
        json!({
            "jsonrpc": "2.0",
            "method": PROCESS_OUTPUT_METHOD,
            "params": {
                "chunk": "b2sK",
                "processId": "proc-1",
                "seq": 1,
                "stream": "stdout"
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "data": "aGk="
            }
        }),
    ]);
    let mut client = EnvironmentDataClient::new(transport);

    client
        .read_file(&ReadFileParams {
            path: EnvironmentPath::new("README.md").expect("path"),
            offset: None,
            max_bytes: None,
        })
        .await
        .expect("response");

    let notification = client
        .next_notification()
        .await
        .expect("notification read")
        .expect("notification");
    assert_eq!(notification.method, PROCESS_OUTPUT_METHOD);
    assert_eq!(notification.params["processId"], "proc-1");
}

#[tokio::test]
async fn data_client_maps_protocol_error_payloads() {
    let transport = MockTransport::with_recv([json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": {
            "code": "notFound",
            "message": "missing"
        }
    })]);
    let mut client = EnvironmentDataClient::new(transport);

    let error = client
        .read_file(&ReadFileParams {
            path: EnvironmentPath::new("missing.txt").expect("path"),
            offset: None,
            max_bytes: None,
        })
        .await
        .expect_err("environment protocol error");

    match error {
        EnvironmentClientError::Protocol(error) => {
            assert_eq!(error.code, EnvironmentProtocolErrorCode::NotFound)
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[tokio::test]
async fn controller_client_sends_typed_initialize_request() {
    let transport = MockTransport::with_recv([json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "capabilities": {
                "closeTarget": true,
                "createTarget": true,
                "getTarget": true,
                "listTargets": true
            },
            "implementation": {
                "name": "test-controller",
                "version": "0.1.0"
            },
            "protocolVersion": 2
        }
    })]);
    let mut client = EnvironmentProviderClient::new(transport);

    let response = client
        .initialize(&ControllerInitializeParams {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            client_name: "lightspeed-test".to_owned(),
        })
        .await
        .expect("response");

    assert!(response.capabilities.create_target);
    assert_eq!(response.implementation.name, "test-controller");

    let transport = client.into_rpc().into_inner();
    assert_eq!(transport.sent[0]["method"], "controller/initialize");
    assert_eq!(transport.sent[0]["params"]["clientName"], "lightspeed-test");
}

#[tokio::test]
async fn controller_client_sends_typed_adoption_request() {
    let transport = MockTransport::with_recv([json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "target": {
                "targetId": "target-adopted",
                "status": "starting",
                "scope": { "type": "default" },
                "capabilities": {}
            }
        }
    })]);
    let mut client = EnvironmentProviderClient::new(transport);

    let response = client
        .adopt_target(&AdoptTargetParams {
            request_id: "adopt-1".to_owned(),
            environment_id: "environment-1".to_owned(),
            incarnation_id: "incarnation-1".to_owned(),
            binding: ProviderBindingContext {
                universe_id: "universe-1".to_owned(),
                binding_id: "primary".to_owned(),
            },
            source_target: "legacy/hand-built-vm".to_owned(),
        })
        .await
        .expect("response");
    assert_eq!(response.target.target_id.as_str(), "target-adopted");

    let transport = client.into_rpc().into_inner();
    assert_eq!(transport.sent[0]["method"], "controller/adoptTarget");
    assert_eq!(
        transport.sent[0]["params"]["sourceTarget"],
        "legacy/hand-built-vm"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn data_client_transfer_methods_preserve_binary_payloads() {
    use environment_protocol::data::transfer::*;
    let entries = vec![TransferEntry {
        path: "".into(),
        content: TransferContent::File {
            data: ByteChunk::from(vec![0, 255, 128]),
            executable: true,
        },
    }];
    let limits = TransferLimits {
        max_entries: 1,
        max_depth: 0,
        max_file_bytes: 3,
        max_total_bytes: 3,
        max_duration_ms: 1000,
    };
    let transport = MockTransport::with_recv([
        json!({"jsonrpc":"2.0", "id":1, "result": {"destination":"/file", "entries":1, "bytes":3, "retiredDirectory":null}}),
        json!({"jsonrpc":"2.0", "id":2, "result": {"source":"/file", "entries":entries, "bytes":3}}),
    ]);
    let mut client = EnvironmentDataClient::new(transport);
    let request = MaterializeParams {
        destination: EnvironmentPath::new("/file").unwrap(),
        entries: entries.clone(),
        limits,
        on_existing: TransferOnExisting::Error,
    };
    assert_eq!(client.materialize(&request).await.unwrap().bytes, 3);
    let captured = client
        .capture(&CaptureParams {
            source: request.destination.clone(),
            limits,
        })
        .await
        .unwrap();
    assert_eq!(captured.entries, entries);
    let transport = client.into_rpc().into_inner();
    assert_eq!(transport.sent[0]["method"], "fs/materialize");
    assert_eq!(
        transport.sent[0]["params"],
        serde_json::to_value(request).unwrap()
    );
    assert_eq!(transport.sent[1]["method"], "fs/capture");
}
