use environment_protocol::error::{EnvironmentProtocolError, EnvironmentProtocolErrorCode};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct RpcRequest {
    pub id: Option<Value>,
    pub method: Option<String>,
    pub params: Value,
}

pub fn parse_request(value: Value) -> Result<RpcRequest, EnvironmentProtocolError> {
    if !value.is_object() {
        return Err(EnvironmentProtocolError::new(
            EnvironmentProtocolErrorCode::InvalidRequest,
            "JSON-RPC request must be an object",
        ));
    }
    let id = value.get("id").cloned();
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    Ok(RpcRequest { id, method, params })
}

pub fn decode_params<T>(params: Value) -> Result<T, EnvironmentProtocolError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(params).map_err(|error| {
        EnvironmentProtocolError::new(
            EnvironmentProtocolErrorCode::InvalidRequest,
            error.to_string(),
        )
    })
}

pub fn encode_result<T>(value: T) -> Result<Value, EnvironmentProtocolError>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|error| {
        EnvironmentProtocolError::new(EnvironmentProtocolErrorCode::Internal, error.to_string())
    })
}

pub fn success_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

pub fn error_response(id: Option<Value>, error: EnvironmentProtocolError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": error
    })
}

pub fn method_not_found(method: &str) -> EnvironmentProtocolError {
    EnvironmentProtocolError::new(
        EnvironmentProtocolErrorCode::Unsupported,
        format!("unsupported environment-protocol method: {method}"),
    )
}

pub fn invalid_request(message: impl Into<String>) -> EnvironmentProtocolError {
    EnvironmentProtocolError::new(EnvironmentProtocolErrorCode::InvalidRequest, message)
}

pub fn not_found(message: impl Into<String>) -> EnvironmentProtocolError {
    EnvironmentProtocolError::new(EnvironmentProtocolErrorCode::NotFound, message)
}

pub fn unsupported(message: impl Into<String>) -> EnvironmentProtocolError {
    EnvironmentProtocolError::new(EnvironmentProtocolErrorCode::Unsupported, message)
}
