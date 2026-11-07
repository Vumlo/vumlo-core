use serde_json::json;

use super::*;

#[test]
fn a_plain_answer_has_text_and_no_tool_calls() {
    let reply = parse(br#"{"choices":[{"message":{"role":"assistant","content":"Hi"}}]}"#).unwrap();
    assert_eq!(reply.content.as_deref(), Some("Hi"));
    assert!(reply.tool_calls.is_empty());
}

#[test]
fn a_null_tool_calls_field_is_none_and_a_local_servers_missing_ids_are_filled_in() {
    let null = parse(br#"{"choices":[{"message":{"content":"x","tool_calls":null}}]}"#).unwrap();
    assert!(null.tool_calls.is_empty());

    let reply = parse(
        br#"{"choices":[{"message":{"content":null,"tool_calls":[
            {"function":{"name":"my_work","arguments":"{}"}},
            {"id":"abc","type":"function","function":{"name":"my_grades"}}]}}]}"#,
    )
    .unwrap();
    let ids: Vec<_> = reply.tool_calls.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["call_0", "abc"]);
    assert_eq!(reply.tool_calls[0].kind, "function");
    assert_eq!(
        reply.tool_calls[1].function.arguments, "",
        "missing arguments are empty text"
    );
}

#[test]
fn an_answer_that_is_not_a_chat_completion_is_malformed() {
    for bad in [
        &b"not json"[..],
        br#"{"choices":[]}"#,
        br#"{"error":"nope"}"#,
    ] {
        assert!(matches!(parse(bad), Err(LlmError::Malformed)), "{bad:?}");
    }
}

#[test]
fn messages_serialize_in_the_openai_shape() {
    let call = ToolCall {
        id: "c1".into(),
        kind: "function".into(),
        function: FunctionCall {
            name: "my_work".into(),
            arguments: "{}".into(),
        },
    };
    let messages = [
        Message::System {
            content: "s".into(),
        },
        Message::User {
            content: "u".into(),
        },
        Message::Assistant {
            content: None,
            tool_calls: vec![call],
        },
        Message::Assistant {
            content: Some("a".into()),
            tool_calls: vec![],
        },
        Message::Tool {
            tool_call_id: "c1".into(),
            content: "{}".into(),
        },
    ];
    let value = serde_json::to_value(messages).unwrap();
    assert_eq!(value[0], json!({"role": "system", "content": "s"}));
    assert_eq!(value[2]["role"], "assistant");
    assert_eq!(value[2]["content"], serde_json::Value::Null);
    assert_eq!(value[2]["tool_calls"][0]["function"]["name"], "my_work");
    assert!(
        value[3].get("tool_calls").is_none(),
        "an empty list is left out"
    );
    assert_eq!(
        value[4],
        json!({"role": "tool", "tool_call_id": "c1", "content": "{}"})
    );
}
