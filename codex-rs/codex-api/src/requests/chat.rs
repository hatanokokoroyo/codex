use crate::error::ApiError;
use crate::provider::Provider;
use crate::requests::headers::build_session_headers;
use crate::requests::headers::insert_header;
use crate::requests::headers::subagent_header;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SessionSource;
use http::HeaderMap;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;

/// Assembled request body plus headers for Chat Completions streaming calls.
pub struct ChatRequest {
    pub body: Value,
    pub headers: HeaderMap,
}

pub struct ChatRequestBuilder<'a> {
    model: &'a str,
    instructions: &'a str,
    input: &'a [ResponseItem],
    tools: &'a [Value],
    conversation_id: Option<String>,
    session_source: Option<SessionSource>,
}

impl<'a> ChatRequestBuilder<'a> {
    pub fn new(
        model: &'a str,
        instructions: &'a str,
        input: &'a [ResponseItem],
        tools: &'a [Value],
    ) -> Self {
        Self {
            model,
            instructions,
            input,
            tools,
            conversation_id: None,
            session_source: None,
        }
    }

    pub fn conversation_id(mut self, id: Option<String>) -> Self {
        self.conversation_id = id;
        self
    }

    pub fn session_source(mut self, source: Option<SessionSource>) -> Self {
        self.session_source = source;
        self
    }

    pub fn build(self, _provider: &Provider) -> Result<ChatRequest, ApiError> {
        let mut messages = Vec::<Value>::new();
        messages.push(json!({"role": "system", "content": self.instructions}));

        let input = self.input;
        let mut reasoning_by_anchor_index: HashMap<usize, String> = HashMap::new();
        let mut last_emitted_role: Option<&str> = None;
        for item in input {
            match item {
                ResponseItem::Message { role, .. } => last_emitted_role = Some(role.as_str()),
                ResponseItem::FunctionCall { .. } | ResponseItem::LocalShellCall { .. } => {
                    last_emitted_role = Some("assistant")
                }
                ResponseItem::FunctionCallOutput { .. } => last_emitted_role = Some("tool"),
                ResponseItem::Reasoning { .. } | ResponseItem::Other => {}
                ResponseItem::CustomToolCall { .. } => {}
                ResponseItem::CustomToolCallOutput { .. } => {}
                ResponseItem::WebSearchCall { .. } => {}
                ResponseItem::Compaction { .. } => {}
                _ => {}
            }
        }

        let mut last_user_index: Option<usize> = None;
        for (idx, item) in input.iter().enumerate() {
            if let ResponseItem::Message { role, .. } = item
                && role == "user"
            {
                last_user_index = Some(idx);
            }
        }

        if !matches!(last_emitted_role, Some("user")) {
            for (idx, item) in input.iter().enumerate() {
                if let Some(u_idx) = last_user_index
                    && idx <= u_idx
                {
                    continue;
                }

                if let ResponseItem::Reasoning {
                    content: Some(items),
                    ..
                } = item
                {
                    let mut text = String::new();
                    for entry in items {
                        match entry {
                            ReasoningItemContent::ReasoningText { text: segment }
                            | ReasoningItemContent::Text { text: segment } => {
                                text.push_str(segment)
                            }
                        }
                    }
                    if text.trim().is_empty() {
                        continue;
                    }

                    let mut attached = false;
                    if idx > 0
                        && let ResponseItem::Message { role, .. } = &input[idx - 1]
                        && role == "assistant"
                    {
                        reasoning_by_anchor_index
                            .entry(idx - 1)
                            .and_modify(|v| v.push_str(&text))
                            .or_insert(text.clone());
                        attached = true;
                    }

                    if !attached && idx + 1 < input.len() {
                        match &input[idx + 1] {
                            ResponseItem::FunctionCall { .. }
                            | ResponseItem::LocalShellCall { .. } => {
                                reasoning_by_anchor_index
                                    .entry(idx + 1)
                                    .and_modify(|v| v.push_str(&text))
                                    .or_insert(text.clone());
                            }
                            ResponseItem::Message { role, .. } if role == "assistant" => {
                                reasoning_by_anchor_index
                                    .entry(idx + 1)
                                    .and_modify(|v| v.push_str(&text))
                                    .or_insert(text.clone());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let mut last_assistant_text: Option<String> = None;

        for (idx, item) in input.iter().enumerate() {
            match item {
                ResponseItem::Message { role, content, .. } => {
                    let mut text = String::new();
                    let mut items: Vec<Value> = Vec::new();
                    let mut saw_image = false;

                    for c in content {
                        match c {
                            ContentItem::InputText { text: t }
                            | ContentItem::OutputText { text: t } => {
                                text.push_str(t);
                                items.push(json!({"type":"text","text": t}));
                            }
                            ContentItem::InputImage { image_url, .. } => {
                                saw_image = true;
                                items.push(
                                    json!({"type":"image_url","image_url": {"url": image_url}}),
                                );
                            }
                            ContentItem::InputAudio { .. } => {}
                            _ => {}
                        }
                    }

                    if role == "assistant" {
                        if let Some(prev) = &last_assistant_text
                            && prev == &text
                        {
                            continue;
                        }
                        last_assistant_text = Some(text.clone());
                    }

                    // If this assistant message appears right after a
                    // tool_calls message (before its tool responses) but has
                    // actual text content, merge the text into the existing
                    // tool_calls message instead of creating a separate one.
                    // If the text is empty, just skip it entirely.
                    if role == "assistant"
                        && messages.last().is_some_and(|m| {
                            m.get("tool_calls").is_some()
                        })
                    {
                        if text.trim().is_empty() {
                            continue;
                        }
                        if let Some(obj) = messages.last_mut().and_then(|m| m.as_object_mut()) {
                            obj.insert("content".to_string(), json!(text));
                            continue;
                        }
                    }

                    let content_value = if role == "assistant" {
                        json!(text)
                    } else if saw_image {
                        json!(items)
                    } else {
                        json!(text)
                    };

                    let chat_role = if role == "developer" {
                        "system"
                    } else {
                        role.as_str()
                    };
                    let mut msg = json!({"role": chat_role, "content": content_value});
                    if role == "assistant"
                        && let Some(reasoning) = reasoning_by_anchor_index.get(&idx)
                        && let Some(obj) = msg.as_object_mut()
                    {
                        obj.insert("reasoning".to_string(), json!(reasoning));
                    }
                    messages.push(msg);
                }
                ResponseItem::FunctionCall {
                    name,
                    arguments,
                    call_id,
                    ..
                } => {
                    let reasoning = reasoning_by_anchor_index.get(&idx).map(String::as_str);
                    let tool_call = json!({
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments,
                        }
                    });
                    push_tool_call_message(&mut messages, tool_call, reasoning);
                }
                ResponseItem::LocalShellCall {
                    call_id,
                    status: _,
                    action,
                    ..
                } => {
                    let reasoning = reasoning_by_anchor_index.get(&idx).map(String::as_str);
                    let tool_call = json!({
                        "id": call_id.clone().unwrap_or_default(),
                        "type": "function",
                        "function": {
                            "name": "shell",
                            "arguments": serde_json::to_value(&action).unwrap_or_default(),
                        }
                    });
                    push_tool_call_message(&mut messages, tool_call, reasoning);
                }
                ResponseItem::FunctionCallOutput {
                    call_id, output, ..
                } => {
                    let content_value =
                        match &output.body {
                            FunctionCallOutputBody::ContentItems(items) => {
                                let mapped: Vec<Value> = items
                                .iter()
                                .map(|it| match it {
                                    FunctionCallOutputContentItem::InputText { text } => {
                                        json!({"type":"text","text": text})
                                    }
                                    FunctionCallOutputContentItem::InputImage { image_url, .. } => {
                                        json!({"type":"image_url","image_url": {"url": image_url}})
                                    }
                                    FunctionCallOutputContentItem::InputAudio { .. }
                                    | FunctionCallOutputContentItem::EncryptedContent { .. } => {
                                        json!(null)
                                    }
                                })
                                .collect();
                                json!(mapped)
                            }
                            FunctionCallOutputBody::Text(content) => json!(content),
                        };

                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": content_value,
                    }));
                }
                ResponseItem::CustomToolCall {
                    call_id,
                    name,
                    input,
                    ..
                } => {
                    let tool_call = json!({
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": input,
                        }
                    });
                    let reasoning = reasoning_by_anchor_index.get(&idx).map(String::as_str);
                    push_tool_call_message(&mut messages, tool_call, reasoning);
                }
                ResponseItem::CustomToolCallOutput {
                    call_id, output, ..
                } => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": output,
                    }));
                }
                ResponseItem::Reasoning { .. }
                | ResponseItem::WebSearchCall { .. }
                | ResponseItem::Other
                | ResponseItem::Compaction { .. }
                | _ => {
                    continue;
                }
            }
        }

        let payload = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "tools": self.tools,
        });

        let mut headers = build_session_headers(self.conversation_id, /*thread_id*/ None);
        if let Some(subagent) = subagent_header(&self.session_source) {
            insert_header(&mut headers, "x-openai-subagent", &subagent);
        }

        Ok(ChatRequest {
            body: payload,
            headers,
        })
    }
}

fn push_tool_call_message(messages: &mut Vec<Value>, tool_call: Value, reasoning: Option<&str>) {
    // Chat Completions requires that tool calls are grouped into a single assistant message
    // (with `tool_calls: [...]`) followed by tool role responses.
    if let Some(Value::Object(obj)) = messages.last_mut()
        && obj.get("role").and_then(Value::as_str) == Some("assistant")
        && obj.get("content").is_some_and(Value::is_null)
        && let Some(tool_calls) = obj.get_mut("tool_calls").and_then(Value::as_array_mut)
    {
        tool_calls.push(tool_call);
        if let Some(reasoning) = reasoning {
            if let Some(Value::String(existing)) = obj.get_mut("reasoning") {
                if !existing.is_empty() {
                    existing.push('\n');
                }
                existing.push_str(reasoning);
            } else {
                obj.insert(
                    "reasoning".to_string(),
                    Value::String(reasoning.to_string()),
                );
            }
        }
        return;
    }

    let mut msg = json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [tool_call],
    });
    if let Some(reasoning) = reasoning
        && let Some(obj) = msg.as_object_mut()
    {
        obj.insert("reasoning".to_string(), json!(reasoning));
    }
    messages.push(msg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::RetryConfig;
    use crate::provider::WireApi;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::protocol::SessionSource;
    use codex_protocol::protocol::SubAgentSource;
    use http::HeaderValue;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    fn provider() -> Provider {
        Provider {
            name: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            query_params: None,
            wire: WireApi::Chat,
            headers: HeaderMap::new(),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(10),
                retry_429: false,
                retry_5xx: true,
                retry_transport: true,
            },
            stream_idle_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn attaches_conversation_and_subagent_headers() {
        let prompt_input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "hi".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }];
        let req = ChatRequestBuilder::new("gpt-test", "inst", &prompt_input, &[])
            .conversation_id(Some("conv-1".into()))
            .session_source(Some(SessionSource::SubAgent(SubAgentSource::Review)))
            .build(&provider())
            .expect("request");

        assert_eq!(
            req.headers.get("session-id"),
            Some(&HeaderValue::from_static("conv-1"))
        );
        assert_eq!(
            req.headers.get("x-openai-subagent"),
            Some(&HeaderValue::from_static("review"))
        );
    }

    #[test]
    fn groups_consecutive_tool_calls_into_a_single_assistant_message() {
        let prompt_input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "read these".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCall {
                name: "read_file".to_string(),
                arguments: r#"{"path":"a.txt"}"#.to_string(),
                call_id: "call-a".to_string(),
                id: None,
                namespace: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCall {
                name: "read_file".to_string(),
                arguments: r#"{"path":"b.txt"}"#.to_string(),
                call_id: "call-b".to_string(),
                id: None,
                namespace: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCall {
                name: "read_file".to_string(),
                arguments: r#"{"path":"c.txt"}"#.to_string(),
                call_id: "call-c".to_string(),
                id: None,
                namespace: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-a".to_string(),
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text("A".to_string()),
                    ..Default::default()
                },
                id: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-b".to_string(),
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text("B".to_string()),
                    ..Default::default()
                },
                id: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-c".to_string(),
                output: FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text("C".to_string()),
                    ..Default::default()
                },
                id: None,
                internal_chat_message_metadata_passthrough: None,
            },
        ];

        let req = ChatRequestBuilder::new("gpt-test", "inst", &prompt_input, &[])
            .build(&provider())
            .expect("request");

        let messages = req
            .body
            .get("messages")
            .and_then(|v| v.as_array())
            .expect("messages array");
        // system + user + assistant(tool_calls=[...]) + 3 tool outputs
        assert_eq!(messages.len(), 6);

        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");

        let tool_calls_msg = &messages[2];
        assert_eq!(tool_calls_msg["role"], "assistant");
        assert_eq!(tool_calls_msg["content"], serde_json::Value::Null);
        let tool_calls = tool_calls_msg["tool_calls"]
            .as_array()
            .expect("tool_calls array");
        assert_eq!(tool_calls.len(), 3);
        assert_eq!(tool_calls[0]["id"], "call-a");
        assert_eq!(tool_calls[1]["id"], "call-b");
        assert_eq!(tool_calls[2]["id"], "call-c");

        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call-a");
        assert_eq!(messages[4]["role"], "tool");
        assert_eq!(messages[4]["tool_call_id"], "call-b");
        assert_eq!(messages[5]["role"], "tool");
        assert_eq!(messages[5]["tool_call_id"], "call-c");
    }
}
