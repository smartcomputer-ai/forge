/// HTTP server mode types (Section 11 MAY feature).
///
/// These types enable Attractor pipelines to be exposed as HTTP endpoints.
/// Currently feature-gated behind `http`.

/// Request to start a pipeline run via HTTP.
#[derive(Clone, Debug)]
pub struct HttpRunRequest {
    pub dot_source: String,
    pub goal: Option<String>,
    pub context: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Response from a pipeline run via HTTP.
#[derive(Clone, Debug)]
pub struct HttpRunResponse {
    pub run_id: String,
    pub status: String,
    pub completed_nodes: Vec<String>,
    pub context: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Server configuration for HTTP mode.
#[derive(Clone, Debug)]
pub struct HttpServerConfig {
    pub bind_address: String,
    pub port: u16,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1".to_string(),
            port: 8080,
        }
    }
}
