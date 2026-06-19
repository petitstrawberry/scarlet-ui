use serde::{Deserialize, Serialize};

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const BUILD_FAILED: i32 = -32001;
pub const PREVIEW_NOT_FOUND: i32 = -32002;
pub const PREVIEW_LIBRARY_LOAD_FAILED: i32 = -32003;
pub const PREVIEW_HOST_WINDOW_ERROR: i32 = -32004;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Params {
    Object(serde_json::Map<String, serde_json::Value>),
    Array(Vec<serde_json::Value>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ErrorObject {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn method_not_found() -> Self {
        Self::new(METHOD_NOT_FOUND, "method not found")
    }

    pub fn invalid_params(message: Option<&str>) -> Self {
        Self::new(INVALID_PARAMS, message.unwrap_or("invalid params"))
    }

    pub fn build_failed(message: &str) -> Self {
        Self::new(BUILD_FAILED, message)
    }

    pub fn preview_not_found(preview: &str) -> Self {
        Self::new(PREVIEW_NOT_FOUND, format!("preview not found: {preview}"))
    }

    pub fn preview_library_load_failed(message: &str) -> Self {
        Self::new(PREVIEW_LIBRARY_LOAD_FAILED, message)
    }

    pub fn preview_host_window_error(message: &str) -> Self {
        Self::new(PREVIEW_HOST_WINDOW_ERROR, message)
    }

    pub fn parse_error() -> Self {
        Self::new(PARSE_ERROR, "parse error")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Params>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    #[serde(default = "default_jsonrpc")]
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Params>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Success {
        jsonrpc: String,
        id: RequestId,
        result: serde_json::Value,
    },
    Error {
        jsonrpc: String,
        id: RequestId,
        error: ErrorObject,
    },
}

impl Response {
    pub fn success(id: RequestId, result: serde_json::Value) -> Self {
        Self::Success {
            jsonrpc: default_jsonrpc(),
            id,
            result,
        }
    }

    pub fn error(id: RequestId, err: ErrorObject) -> Self {
        Self::Error {
            jsonrpc: default_jsonrpc(),
            id,
            error: err,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request(Request),
    Notification(Notification),
}

pub fn parse_message(line: &str) -> Result<Incoming, ErrorObject> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|_| ErrorObject::parse_error())?;
    let Some(object) = value.as_object() else {
        return Err(ErrorObject::new(INVALID_REQUEST, "invalid request"));
    };

    match object.get("method") {
        Some(serde_json::Value::String(_)) => {}
        Some(_) | None => return Err(ErrorObject::new(INVALID_REQUEST, "invalid request")),
    }

    if object.contains_key("id") {
        serde_json::from_value(value)
            .map(Incoming::Request)
            .map_err(|_| ErrorObject::new(INVALID_REQUEST, "invalid request"))
    } else {
        serde_json::from_value(value)
            .map(Incoming::Notification)
            .map_err(|_| ErrorObject::new(INVALID_REQUEST, "invalid request"))
    }
}

pub fn serialize_message<T: Serialize>(msg: &T) -> Result<String, ErrorObject> {
    let line = serde_json::to_string(msg)
        .map_err(|error| ErrorObject::new(INVALID_REQUEST, error.to_string()))?;
    assert!(!line.contains('\n'));
    Ok(line)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewDescriptorJson {
    pub id: String,
    pub name: String,
}

impl PreviewDescriptorJson {
    pub fn from_descriptor(d: &scarlet_ui::preview::PreviewDescriptor) -> Self {
        Self {
            id: d.id.as_str().to_string(),
            name: d.name.clone(),
        }
    }
}

fn default_jsonrpc() -> String {
    String::from("2.0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_message_valid_request_with_string_id() {
        let incoming =
            parse_message(r#"{"jsonrpc":"2.0","id":"abc","method":"initialize","params":{}}"#)
                .unwrap();

        match incoming {
            Incoming::Request(request) => {
                assert_eq!(request.id, RequestId::String(String::from("abc")));
                assert_eq!(request.method, "initialize");
            }
            Incoming::Notification(_) => panic!("expected request"),
        }
    }

    #[test]
    fn parse_message_valid_request_with_numeric_id() {
        let incoming =
            parse_message(r#"{"jsonrpc":"2.0","id":42,"method":"preview/list"}"#).unwrap();

        match incoming {
            Incoming::Request(request) => {
                assert_eq!(request.id, RequestId::Number(42));
                assert_eq!(request.method, "preview/list");
            }
            Incoming::Notification(_) => panic!("expected request"),
        }
    }

    #[test]
    fn parse_message_valid_notification() {
        let incoming =
            parse_message(r#"{"jsonrpc":"2.0","method":"workspace/didChangeActiveFile"}"#).unwrap();

        match incoming {
            Incoming::Notification(notification) => {
                assert_eq!(notification.method, "workspace/didChangeActiveFile");
                assert!(notification.params.is_none());
            }
            Incoming::Request(_) => panic!("expected notification"),
        }
    }

    #[test]
    fn parse_message_malformed_json_returns_parse_error() {
        let error = parse_message(r#"{"jsonrpc":"2.0","method":"initialize""#).unwrap_err();

        assert_eq!(error.code, PARSE_ERROR);
    }

    #[test]
    fn parse_message_missing_method_returns_invalid_request() {
        let error = parse_message(r#"{"jsonrpc":"2.0","id":1}"#).unwrap_err();

        assert_eq!(error.code, INVALID_REQUEST);
    }

    #[test]
    fn parse_message_non_string_method_returns_invalid_request() {
        let error = parse_message(r#"{"jsonrpc":"2.0","id":1,"method":7}"#).unwrap_err();

        assert_eq!(error.code, INVALID_REQUEST);
    }

    #[test]
    fn response_success_serializes_with_result_and_no_error() {
        let response = Response::success(RequestId::Number(1), json!({ "ok": true }));
        let value: serde_json::Value =
            serde_json::from_str(&serialize_message(&response).unwrap()).unwrap();

        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["result"], json!({ "ok": true }));
        assert!(value.get("error").is_none());
    }

    #[test]
    fn response_error_serializes_with_error_and_no_result() {
        let response = Response::error(
            RequestId::String(String::from("1")),
            ErrorObject::method_not_found(),
        );
        let value: serde_json::Value =
            serde_json::from_str(&serialize_message(&response).unwrap()).unwrap();

        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["error"]["code"], METHOD_NOT_FOUND);
        assert!(value.get("result").is_none());
    }

    #[test]
    fn error_object_domain_constructors_use_expected_codes() {
        assert_eq!(ErrorObject::method_not_found().code, METHOD_NOT_FOUND);
        assert_eq!(ErrorObject::invalid_params(None).code, INVALID_PARAMS);
        assert_eq!(ErrorObject::build_failed("build failed").code, BUILD_FAILED);
        assert_eq!(
            ErrorObject::preview_not_found("preview").code,
            PREVIEW_NOT_FOUND
        );
        assert_eq!(
            ErrorObject::preview_library_load_failed("load failed").code,
            PREVIEW_LIBRARY_LOAD_FAILED
        );
        assert_eq!(
            ErrorObject::preview_host_window_error("window failed").code,
            PREVIEW_HOST_WINDOW_ERROR
        );
        assert_eq!(ErrorObject::parse_error().code, PARSE_ERROR);
    }

    #[test]
    fn serialize_message_produces_single_line() {
        let response =
            Response::success(RequestId::Number(1), json!({ "message": "hello\nworld" }));
        let line = serialize_message(&response).unwrap();

        assert!(!line.contains('\n'));
    }

    #[test]
    fn request_round_trips_through_serialize_and_parse() {
        let request = Request {
            jsonrpc: default_jsonrpc(),
            id: RequestId::String(String::from("req-1")),
            method: String::from("preview/list"),
            params: Some(Params::Object(serde_json::Map::new())),
        };

        let line = serialize_message(&request).unwrap();
        let incoming = parse_message(&line).unwrap();

        assert_eq!(incoming, Incoming::Request(request));
    }

    #[test]
    fn preview_descriptor_json_maps_id_and_name() {
        let descriptor = scarlet_ui::preview::PreviewDescriptor::new(
            scarlet_ui::preview::PreviewId::new("counter_preview"),
            "Counter Preview",
        );

        let json = PreviewDescriptorJson::from_descriptor(&descriptor);

        assert_eq!(json.id, "counter_preview");
        assert_eq!(json.name, "Counter Preview");
    }
}
