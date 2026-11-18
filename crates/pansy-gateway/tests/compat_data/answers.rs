//! Written by hand as each vendor sends it; the streams are spelled out
//! frame by frame.

use pansy_gateway::ir::{Part, Response, Stop};

use super::{Answer, need};

fn text(r: &Response) -> String {
    r.parts
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

fn text_and_cost(r: &Response) -> Result<(), String> {
    need(text(r) == "Hello there", "the text changed")?;
    need(r.usage.output == 5, "the tokens out changed")?;
    need(
        r.usage.input == 14,
        "the tokens in changed (14, the cache's 4 among them)",
    )?;
    need(
        r.usage.cache_read == 4,
        "the tokens read from the cache changed",
    )?;
    need(r.stop == Stop::End, "it no longer ends as finished")
}

fn tool_call(r: &Response) -> Result<(), String> {
    let call = r
        .parts
        .iter()
        .find_map(|p| match p {
            Part::ToolCall(c) => Some(c),
            _ => None,
        })
        .ok_or("the call is gone")?;
    need(call.name == "get_weather", "the call's name changed")?;
    let args: serde_json::Value =
        serde_json::from_str(&call.arguments).map_err(|_| "the call's arguments are not JSON")?;
    need(args["city"] == "Paris", "the call's arguments changed")?;
    need(r.stop == Stop::ToolUse, "it no longer ends as a tool call")
}

fn reasoning_then_text(r: &Response) -> Result<(), String> {
    let thought: String = r
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Thinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    need(thought == "Let me think.", "the reasoning is gone")?;
    need(text(r) == "Hello there", "the text changed")?;
    let first = r
        .parts
        .iter()
        .position(|p| matches!(p, Part::Thinking { .. }));
    let then = r.parts.iter().position(|p| matches!(p, Part::Text(_)));
    need(first < then, "the reasoning no longer comes first")
}

fn cut_off(r: &Response) -> Result<(), String> {
    need(text(r) == "Hello there", "the text changed")?;
    need(r.stop == Stop::MaxTokens, "it no longer ends as cut off")
}

pub const ANSWERS: &[Answer] = &[
    Answer {
        title: "Text, and what it cost",
        whole: [
            r##"{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[{"type":"text","text":"Hello there"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":4}}"##,
            r##"{"id":"c1","object":"chat.completion","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"Hello there"},"finish_reason":"stop"}],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}"##,
            r##"{"id":"resp_1","object":"response","status":"completed","model":"m","output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}}}"##,
            r##"{"candidates":[{"content":{"role":"model","parts":[{"text":"Hello there"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4},"modelVersion":"m"}"##,
        ],
        stream: [
            r##"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":10,"cache_read_input_tokens":4,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"there"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"##,
            r##"data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello "},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"content":"there"},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}

data: [DONE]

"##,
            r##"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","object":"response","status":"in_progress","model":"m","output":[]}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","status":"in_progress","content":[]}}

event: response.content_part.added
data: {"type":"response.content_part.added","item_id":"msg_1","content_index":0,"part":{"type":"output_text","text":"","annotations":[]},"output_index":0}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"Hello ","output_index":0}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"there","output_index":0}

event: response.output_text.done
data: {"type":"response.output_text.done","item_id":"msg_1","content_index":0,"text":"Hello there","output_index":0}

event: response.content_part.done
data: {"type":"response.content_part.done","item_id":"msg_1","content_index":0,"part":{"type":"output_text","text":"Hello there","annotations":[]},"output_index":0}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","object":"response","status":"completed","model":"m","output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}}}}

"##,
            r##"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hello "}]},"index":0}],"modelVersion":"m"}

data: {"candidates":[{"content":{"role":"model","parts":[{"text":"there"}]},"index":0,"finishReason":"STOP"}],"modelVersion":"m","usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4}}

"##,
        ],
        wire: [
            &[
                ("/content/0/text", "Hello there"),
                ("/usage/output_tokens", "5"),
            ],
            &[
                ("/choices/0/message/content", "Hello there"),
                ("/usage/completion_tokens", "5"),
                ("/usage/prompt_tokens", "14"),
            ],
            &[
                ("/output", "Hello there"),
                ("/usage/output_tokens", "5"),
                ("/usage/input_tokens", "14"),
            ],
            &[
                ("/candidates/0/content/parts", "Hello there"),
                ("/usageMetadata/candidatesTokenCount", "5"),
                ("/usageMetadata/promptTokenCount", "14"),
            ],
        ],
        check: text_and_cost,
    },
    Answer {
        title: "A tool call",
        whole: [
            r##"{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[{"type":"tool_use","id":"toolu_1","name":"get_weather","input":{"city":"Paris"}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":4}}"##,
            r##"{"id":"c1","object":"chat.completion","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}"##,
            r##"{"id":"resp_1","object":"response","status":"completed","model":"m","output":[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}","status":"completed"}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}}}"##,
            r##"{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4},"modelVersion":"m"}"##,
        ],
        stream: [
            r##"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":10,"cache_read_input_tokens":4,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_weather","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"city\""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":":\"Paris\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"##,
            r##"data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\""}}]},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"Paris\"}"}}]},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}

data: [DONE]

"##,
            r##"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","object":"response","status":"in_progress","model":"m","output":[]}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather","arguments":"","status":"in_progress"}}

event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"city\"","output_index":0}

event: response.function_call_arguments.delta
data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":":\"Paris\"}","output_index":0}

event: response.function_call_arguments.done
data: {"type":"response.function_call_arguments.done","item_id":"fc_1","arguments":"{\"city\":\"Paris\"}","output_index":0}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}","status":"completed"}}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","object":"response","status":"completed","model":"m","output":[{"type":"function_call","id":"fc_1","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}","status":"completed"}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}}}}

"##,
            r##"data: {"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},"index":0,"finishReason":"STOP"}],"modelVersion":"m","usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4}}

"##,
        ],
        wire: [
            &[
                ("/content", "tool_use"),
                ("/content", "Paris"),
                ("/stop_reason", "tool_use"),
            ],
            &[
                (
                    "/choices/0/message/tool_calls/0/function/name",
                    "get_weather",
                ),
                (
                    "/choices/0/message/tool_calls/0/function/arguments",
                    "Paris",
                ),
                ("/choices/0/finish_reason", "tool_calls"),
            ],
            &[("/output", "function_call"), ("/output", "Paris")],
            &[
                ("/candidates/0/content/parts", "functionCall"),
                ("/candidates/0/content/parts", "Paris"),
            ],
        ],
        check: tool_call,
    },
    Answer {
        title: "Reasoning, then text",
        whole: [
            r##"{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[{"type":"thinking","thinking":"Let me think.","signature":"sig"},{"type":"text","text":"Hello there"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":4}}"##,
            r##"{"id":"c1","object":"chat.completion","model":"m","choices":[{"index":0,"message":{"role":"assistant","reasoning_content":"Let me think.","content":"Hello there"},"finish_reason":"stop"}],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}"##,
            r##"{"id":"resp_1","object":"response","status":"completed","model":"m","output":[{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Let me think."}]},{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}}}"##,
            r##"{"candidates":[{"content":{"role":"model","parts":[{"text":"Let me think.","thought":true},{"text":"Hello there"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4},"modelVersion":"m"}"##,
        ],
        stream: [
            r##"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":10,"cache_read_input_tokens":4,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hello there"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"##,
            r##"data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"Let me "},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"reasoning_content":"think."},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"content":"Hello there"},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}

data: [DONE]

"##,
            r##"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","object":"response","status":"in_progress","model":"m","output":[]}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[]}}

event: response.reasoning_summary_part.added
data: {"type":"response.reasoning_summary_part.added","item_id":"rs_1","summary_index":0,"part":{"type":"summary_text","text":""},"output_index":0}

event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"Let me ","output_index":0}

event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","item_id":"rs_1","summary_index":0,"delta":"think.","output_index":0}

event: response.reasoning_summary_text.done
data: {"type":"response.reasoning_summary_text.done","item_id":"rs_1","summary_index":0,"text":"Let me think.","output_index":0}

event: response.reasoning_summary_part.done
data: {"type":"response.reasoning_summary_part.done","item_id":"rs_1","summary_index":0,"part":{"type":"summary_text","text":"Let me think."},"output_index":0}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Let me think."}]}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"msg_1","role":"assistant","status":"in_progress","content":[]}}

event: response.content_part.added
data: {"type":"response.content_part.added","item_id":"msg_1","content_index":0,"part":{"type":"output_text","text":"","annotations":[]},"output_index":1}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"Hello there","output_index":1}

event: response.output_text.done
data: {"type":"response.output_text.done","item_id":"msg_1","content_index":0,"text":"Hello there","output_index":1}

event: response.content_part.done
data: {"type":"response.content_part.done","item_id":"msg_1","content_index":0,"part":{"type":"output_text","text":"Hello there","annotations":[]},"output_index":1}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":1,"item":{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","object":"response","status":"completed","model":"m","output":[{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"Let me think."}]},{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}}}}

"##,
            r##"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Let me think.","thought":true}]},"index":0}],"modelVersion":"m"}

data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hello there"}]},"index":0,"finishReason":"STOP"}],"modelVersion":"m","usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4}}

"##,
        ],
        wire: [
            &[("/content", "Let me think."), ("/content", "Hello there")],
            &[
                ("/choices/0/message", "Let me think."),
                ("/choices/0/message/content", "Hello there"),
            ],
            &[("/output", "Let me think."), ("/output", "Hello there")],
            &[
                ("/candidates/0/content/parts", "Let me think."),
                ("/candidates/0/content/parts", "Hello there"),
            ],
        ],
        check: reasoning_then_text,
    },
    Answer {
        title: "An answer cut off at its longest",
        whole: [
            r##"{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[{"type":"text","text":"Hello there"}],"stop_reason":"max_tokens","stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":4}}"##,
            r##"{"id":"c1","object":"chat.completion","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"Hello there"},"finish_reason":"length"}],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}"##,
            r##"{"id":"resp_1","object":"response","status":"incomplete","model":"m","output":[{"type":"message","id":"msg_1","role":"assistant","status":"incomplete","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}},"incomplete_details":{"reason":"max_output_tokens"}}"##,
            r##"{"candidates":[{"content":{"role":"model","parts":[{"text":"Hello there"}]},"finishReason":"MAX_TOKENS","index":0}],"usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4},"modelVersion":"m"}"##,
        ],
        stream: [
            r##"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[],"stop_reason":null,"usage":{"input_tokens":10,"cache_read_input_tokens":4,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello there"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"max_tokens","stop_sequence":null},"usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"##,
            r##"data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello there"},"finish_reason":null}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[{"index":0,"delta":{},"finish_reason":"length"}]}

data: {"id":"c1","object":"chat.completion.chunk","model":"m","choices":[],"usage":{"prompt_tokens":14,"completion_tokens":5,"total_tokens":19,"prompt_tokens_details":{"cached_tokens":4}}}

data: [DONE]

"##,
            r##"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","object":"response","status":"in_progress","model":"m","output":[]}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","status":"in_progress","content":[]}}

event: response.content_part.added
data: {"type":"response.content_part.added","item_id":"msg_1","content_index":0,"part":{"type":"output_text","text":"","annotations":[]},"output_index":0}

event: response.output_text.delta
data: {"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"Hello there","output_index":0}

event: response.output_text.done
data: {"type":"response.output_text.done","item_id":"msg_1","content_index":0,"text":"Hello there","output_index":0}

event: response.content_part.done
data: {"type":"response.content_part.done","item_id":"msg_1","content_index":0,"part":{"type":"output_text","text":"Hello there","annotations":[]},"output_index":0}

event: response.output_item.done
data: {"type":"response.output_item.done","output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}}

event: response.incomplete
data: {"type":"response.incomplete","response":{"id":"resp_1","object":"response","status":"incomplete","model":"m","output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello there","annotations":[]}]}],"usage":{"input_tokens":14,"output_tokens":5,"total_tokens":19,"input_tokens_details":{"cached_tokens":4}},"incomplete_details":{"reason":"max_output_tokens"}}}

"##,
            r##"data: {"candidates":[{"content":{"role":"model","parts":[{"text":"Hello there"}]},"index":0,"finishReason":"MAX_TOKENS"}],"modelVersion":"m","usageMetadata":{"promptTokenCount":14,"candidatesTokenCount":5,"totalTokenCount":19,"cachedContentTokenCount":4}}

"##,
        ],
        wire: [
            &[("/stop_reason", "max_tokens")],
            &[("/choices/0/finish_reason", "length")],
            &[("/status", "incomplete")],
            &[("/candidates/0/finishReason", "MAX_TOKENS")],
        ],
        check: cut_off,
    },
];
