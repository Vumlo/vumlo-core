//! The OpenAI-compatible chat endpoint (ADR-0070): `POST {endpoint}/chat/completions` with tools.
//! One request, one whole answer: `OutboundClient` buffers the response, so there is no streaming.
//! A body is never logged; only a status is.

use std::sync::OnceLock;
use std::time::Duration;

use module_api::{OutboundClient, OutboundError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A model on the school's own server can take a while.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_RESPONSE_BYTES: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error(transparent)]
    Outbound(#[from] OutboundError),
    #[error("the model answered HTTP {0}")]
    Status(u16),
    #[error("the model's answer is not a chat completion")]
    Malformed,
}

/// Where and as whom to ask.
pub struct Model<'a> {
    pub endpoint: &'a str,
    pub name: &'a str,
    pub api_key: Option<&'a str>,
}

/// One entry of the conversation sent to the model.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Some local servers leave it out; [`chat`] fills it in.
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "function_kind")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// A JSON document in a string, as the model wrote it: untrusted until parsed.
    #[serde(default)]
    pub arguments: String,
}

fn function_kind() -> String {
    "function".to_owned()
}

/// A tool as the model is told about it.
#[derive(Debug, Serialize)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    kind: &'static str,
    function: FunctionSpec,
}

#[derive(Debug, Serialize)]
struct FunctionSpec {
    name: &'static str,
    description: &'static str,
    /// A JSON Schema object.
    parameters: Value,
}

impl ToolSpec {
    pub fn new(name: &'static str, description: &'static str, parameters: Value) -> Self {
        Self {
            kind: "function",
            function: FunctionSpec {
                name,
                description,
                parameters,
            },
        }
    }

    pub fn name(&self) -> &'static str {
        self.function.name
    }
}

/// What the model answered: text, tool calls, or both.
#[derive(Debug, Default, Deserialize)]
pub struct Reply {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub tool_calls: Vec<ToolCall>,
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<ToolCall>, D::Error> {
    Ok(Option::<Vec<ToolCall>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    messages: &'a [Message],
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    tools: &'a [ToolSpec],
}

#[derive(Deserialize)]
struct Response {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Reply,
}

static CLIENT: OnceLock<OutboundClient> = OnceLock::new();

fn client() -> Result<&'static OutboundClient, LlmError> {
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    // No redirects: the key must never be sent anywhere but the endpoint the admin wrote.
    let built = OutboundClient::new(CONNECT_TIMEOUT, TOTAL_TIMEOUT, MAX_RESPONSE_BYTES, 0)?;
    Ok(CLIENT.get_or_init(|| built))
}

pub async fn chat(
    model: &Model<'_>,
    messages: &[Message],
    tools: &[ToolSpec],
) -> Result<Reply, LlmError> {
    let body = serde_json::to_vec(&Request {
        model: model.name,
        messages,
        tools,
    })
    .map_err(|_| LlmError::Malformed)?;
    let url = format!("{}/chat/completions", model.endpoint.trim_end_matches('/'));
    let authorization = model.api_key.map(|key| format!("Bearer {key}"));
    let mut headers = vec![("content-type", "application/json")];
    if let Some(value) = &authorization {
        headers.push(("authorization", value.as_str()));
    }
    let (status, bytes) = client()?.post(&url, &headers, body).await?;
    if !(200..300).contains(&status) {
        tracing::warn!(status, "ai_agent: the model refused the request");
        return Err(LlmError::Status(status));
    }
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> Result<Reply, LlmError> {
    let response: Response = serde_json::from_slice(bytes).map_err(|_| LlmError::Malformed)?;
    let mut reply = response
        .choices
        .into_iter()
        .next()
        .ok_or(LlmError::Malformed)?
        .message;
    for (n, call) in reply.tool_calls.iter_mut().enumerate() {
        if call.id.is_empty() {
            call.id = format!("call_{n}");
        }
    }
    Ok(reply)
}

#[cfg(test)]
mod tests;
