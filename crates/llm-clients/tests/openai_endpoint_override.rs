use std::collections::BTreeMap;

use llm_clients::openai::{completions, responses};
use llm_clients::{EndpointOverride, RequestAuth};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

async fn one_request_server(
    body: &'static str,
) -> (
    String,
    oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let (sender, receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).await.expect("read");
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(headers_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..headers_end + 4]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if bytes.len() >= headers_end + 4 + content_length {
                    break;
                }
            }
        }
        let request = String::from_utf8(bytes).expect("UTF-8 request");
        let _ = sender.send(request);
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.expect("write");
    });
    (format!("http://{address}/v1"), receiver, task)
}

#[tokio::test(flavor = "current_thread")]
async fn completions_override_routes_auth_and_custom_headers_without_openai_defaults() {
    let (base_url, request, server) = one_request_server(
        r#"{"id":"chatcmpl_test","object":"chat.completion","created":1,"model":"router-model","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]}"#,
    )
    .await;
    let mut config = completions::Config::new("deployment-openai-key");
    config.organization = Some("org-must-not-leak".to_owned());
    config.project = Some("project-must-not-leak".to_owned());
    let client = completions::Client::new(config).expect("client");
    let endpoint = EndpointOverride::from_parts(
        &base_url,
        &BTreeMap::from([("x-title".to_owned(), "Lightspeed".to_owned())]),
    )
    .expect("endpoint");

    client
        .create_with_transport(
            completions::CreateCompletionRequest::user_text("router-model", "hello"),
            Some(RequestAuth::ApiKey("universe-router-key")),
            Some(&endpoint),
        )
        .await
        .expect("completion");
    let request = request
        .await
        .expect("captured request")
        .to_ascii_lowercase();
    assert!(request.starts_with("post /v1/chat/completions http/1.1"));
    assert!(request.contains("authorization: bearer universe-router-key"));
    assert!(request.contains("x-title: lightspeed"));
    assert!(!request.contains("org-must-not-leak"));
    assert!(!request.contains("project-must-not-leak"));
    server.await.expect("server");
}

#[tokio::test(flavor = "current_thread")]
async fn anonymous_override_does_not_fall_back_to_the_client_api_key() {
    let (base_url, request, server) = one_request_server(
        r#"{"object":"list","data":[{"id":"local-model","object":"model","created":1,"owned_by":"local"}]}"#,
    )
    .await;
    let client =
        completions::Client::new(completions::Config::new("must-not-leak")).expect("client");
    let endpoint = EndpointOverride::from_parts(&base_url, &BTreeMap::new()).expect("endpoint");

    let models = client
        .list_models_with_transport(Some(RequestAuth::None), Some(&endpoint))
        .await
        .expect("models");
    assert_eq!(models.parsed.data[0].id, "local-model");
    let request = request
        .await
        .expect("captured request")
        .to_ascii_lowercase();
    assert!(request.starts_with("get /v1/models http/1.1"));
    assert!(!request.contains("authorization:"));
    assert!(!request.contains("must-not-leak"));
    server.await.expect("server");
}

#[tokio::test(flavor = "current_thread")]
async fn responses_override_uses_the_responses_path() {
    let (base_url, request, server) = one_request_server(
        r#"{"id":"resp_test","object":"response","status":"completed","output":[]}"#,
    )
    .await;
    let client = responses::Client::new(responses::Config::without_api_key()).expect("client");
    let endpoint = EndpointOverride::from_parts(&base_url, &BTreeMap::new()).expect("endpoint");
    let response = client
        .create_with_transport(
            responses::CreateResponseRequest::text("router-model", "hello"),
            Some(RequestAuth::Bearer("oauth-token")),
            Some(&endpoint),
        )
        .await
        .expect("response");
    assert_eq!(response.parsed.id, "resp_test");
    let request = request
        .await
        .expect("captured request")
        .to_ascii_lowercase();
    assert!(request.starts_with("post /v1/responses http/1.1"));
    assert!(request.contains("authorization: bearer oauth-token"));
    server.await.expect("server");
}

#[tokio::test(flavor = "current_thread")]
async fn anonymous_auth_without_an_endpoint_is_rejected_before_io() {
    let completions = completions::Client::new(completions::Config::new("deployment-key"))
        .expect("completions client");
    let error = completions
        .list_models_with_transport(Some(RequestAuth::None), None)
        .await
        .expect_err("anonymous default OpenAI request must fail");
    assert!(matches!(error, llm_clients::LlmApiError::Configuration(_)));

    let responses =
        responses::Client::new(responses::Config::without_api_key()).expect("responses client");
    let error = responses
        .list_models_with_transport(Some(RequestAuth::None), None)
        .await
        .expect_err("anonymous default OpenAI request must fail");
    assert!(matches!(error, llm_clients::LlmApiError::Configuration(_)));
}

#[test]
fn endpoint_override_rejects_public_http_credentials_and_reserved_headers() {
    EndpointOverride::from_parts("http://[::1]:11434/v1", &BTreeMap::new())
        .expect("IPv6 loopback endpoint");
    for url in [
        "http://router.example/v1",
        "https://user:secret@router.example/v1",
        "https://router.example/v1?api-version=1",
    ] {
        assert!(
            EndpointOverride::from_parts(url, &BTreeMap::new()).is_err(),
            "{url}"
        );
    }
    assert!(
        EndpointOverride::from_parts(
            "https://router.example/v1",
            &BTreeMap::from([("authorization".to_owned(), "secret".to_owned())]),
        )
        .is_err()
    );
}
