use pansy_gateway::ir::{Image, Part, Request, Role, ToolChoice};

use super::{Ask, need};

const NONE: [Option<&str>; 4] = [None; 4];

fn parts(r: &Request) -> impl Iterator<Item = &Part> {
    r.messages.iter().flat_map(|m| m.parts.iter())
}

/// A request with the weather tool, as protocol A, C, R or G declares it.
macro_rules! with {
    (A, $rest:literal) => {
        concat!("{", r#""tools":[{"name":"get_weather","description":"The weather in a city","input_schema":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}]"#, ",", $rest, "}")
    };
    (C, $rest:literal) => {
        concat!("{", r#""tools":[{"type":"function","function":{"name":"get_weather","description":"The weather in a city","parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}}]"#, ",", $rest, "}")
    };
    (R, $rest:literal) => {
        concat!("{", r#""tools":[{"type":"function","name":"get_weather","description":"The weather in a city","parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}]"#, ",", $rest, "}")
    };
    (G, $rest:literal) => {
        concat!("{", r#""tools":[{"functionDeclarations":[{"name":"get_weather","description":"The weather in a city","parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}]}]"#, ",", $rest, "}")
    };
}

pub const REQUESTS: &[Ask] = &[
    Ask {
        title: "System instructions",
        samples: [
            Some(
                r#"{"model":"m","max_tokens":1024,"system":"Be brief.","messages":[{"role":"user","content":"hi"}]}"#,
            ),
            Some(
                r#"{"model":"m","messages":[{"role":"system","content":"Be brief."},{"role":"user","content":"hi"}]}"#,
            ),
            Some(
                r#"{"model":"m","instructions":"Be brief.","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]}"#,
            ),
            Some(
                r#"{"systemInstruction":{"parts":[{"text":"Be brief."}]},"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#,
            ),
        ],
        no_vendor: NONE,
        wire: [
            &[("/system", "Be brief.")],
            &[("/messages/0", "Be brief.")],
            &[("/instructions", "Be brief.")],
            &[("/systemInstruction", "Be brief.")],
        ],
        wire_only: false,
        check: |r| {
            need(
                r.system.join("\n").contains("Be brief."),
                "the instructions are gone",
            )
        },
    },
    Ask {
        title: "A conversation of several turns",
        samples: [
            Some(
                r#"{"model":"m","max_tokens":1024,"messages":[{"role":"user","content":"one"},{"role":"assistant","content":"two"},{"role":"user","content":"three"}]}"#,
            ),
            Some(
                r#"{"model":"m","messages":[{"role":"user","content":"one"},{"role":"assistant","content":"two"},{"role":"user","content":"three"}]}"#,
            ),
            Some(
                r#"{"model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"one"}]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"two"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"three"}]}]}"#,
            ),
            Some(
                r#"{"contents":[{"role":"user","parts":[{"text":"one"}]},{"role":"model","parts":[{"text":"two"}]},{"role":"user","parts":[{"text":"three"}]}]}"#,
            ),
        ],
        no_vendor: NONE,
        wire: [
            &[("/messages/1/role", "assistant"), ("/messages/2", "three")],
            &[("/messages/1/role", "assistant"), ("/messages/2", "three")],
            &[("/input/1", "assistant"), ("/input/2", "three")],
            &[("/contents/1/role", "model"), ("/contents/2", "three")],
        ],
        wire_only: false,
        check: |r| {
            let roles: Vec<Role> = r.messages.iter().map(|m| m.role).collect();
            need(
                roles == [Role::User, Role::Assistant, Role::User],
                "the turns changed",
            )
        },
    },
    Ask {
        title: "Tools the model may call",
        samples: [
            Some(with!(
                A,
                r#""model":"m","max_tokens":1024,"messages":[{"role":"user","content":"hi"}]"#
            )),
            Some(with!(
                C,
                r#""model":"m","messages":[{"role":"user","content":"hi"}]"#
            )),
            Some(with!(
                R,
                r#""model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]"#
            )),
            Some(with!(
                G,
                r#""contents":[{"role":"user","parts":[{"text":"hi"}]}]"#
            )),
        ],
        no_vendor: NONE,
        wire: [
            &[
                ("/tools/0/name", "get_weather"),
                ("/tools/0/input_schema", "city"),
            ],
            &[
                ("/tools/0/function/name", "get_weather"),
                ("/tools/0/function/parameters", "city"),
            ],
            &[
                ("/tools/0/name", "get_weather"),
                ("/tools/0/parameters", "city"),
            ],
            &[
                ("/tools/0/functionDeclarations/0/name", "get_weather"),
                ("/tools/0/functionDeclarations/0", "city"),
            ],
        ],
        wire_only: false,
        check: |r| {
            let tool = r.tools.first().ok_or("no tool")?;
            need(tool.name == "get_weather", "the tool's name changed")?;
            need(
                tool.parameters.to_string().contains("city"),
                "its parameters are gone",
            )?;
            need(
                tool.description.as_deref() == Some("The weather in a city"),
                "its description is gone",
            )
        },
    },
    Ask {
        title: "A tool call and its result, earlier in the conversation",
        samples: [
            Some(with!(
                A,
                r#""model":"m","max_tokens":1024,"messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"get_weather","input":{"city":"Paris"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"18C"}]}]"#
            )),
            Some(with!(
                C,
                r#""model":"m","messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"18C"}]"#
            )),
            Some(with!(
                R,
                r#""model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"weather?"}]},{"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}"},{"type":"function_call_output","call_id":"call_1","output":"18C"}]"#
            )),
            Some(with!(
                G,
                r#""contents":[{"role":"user","parts":[{"text":"weather?"}]},{"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},{"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"result":"18C"}}}]}]"#
            )),
        ],
        no_vendor: NONE,
        wire: [
            &[
                ("/messages/1", "tool_use"),
                ("/messages/2", "tool_result"),
                ("/messages/2", "18C"),
            ],
            &[
                ("/messages/1/tool_calls/0/function/name", "get_weather"),
                ("/messages/2/role", "tool"),
                ("/messages/2", "18C"),
            ],
            &[
                ("/input/1/type", "function_call"),
                ("/input/2/type", "function_call_output"),
                ("/input/2", "18C"),
            ],
            &[
                ("/contents/1", "functionCall"),
                ("/contents/2", "functionResponse"),
                ("/contents/2", "18C"),
            ],
        ],
        wire_only: false,
        check: |r| {
            let call = parts(r)
                .find_map(|p| match p {
                    Part::ToolCall(c) => Some(c),
                    _ => None,
                })
                .ok_or("the call is gone")?;
            need(call.name == "get_weather", "the call's name changed")?;
            need(
                call.arguments.contains("Paris"),
                "the call's arguments are gone",
            )?;
            let result = parts(r)
                .find_map(|p| match p {
                    Part::ToolResult(t) => Some(t),
                    _ => None,
                })
                .ok_or("the result is gone")?;
            need(result.text().contains("18C"), "the result's text is gone")?;
            need(
                result.id == call.id,
                "the result no longer answers the call",
            )
        },
    },
    Ask {
        title: "Two tool calls in one turn, and their results",
        samples: [
            Some(with!(
                A,
                r#""model":"m","max_tokens":1024,"messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":[{"type":"tool_use","id":"call_1","name":"get_weather","input":{"city":"Paris"}},{"type":"tool_use","id":"call_2","name":"get_weather","input":{"city":"Rome"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"18C"},{"type":"tool_result","tool_use_id":"call_2","content":"24C"}]}]"#
            )),
            Some(with!(
                C,
                r#""model":"m","messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}},{"id":"call_2","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Rome\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"18C"},{"role":"tool","tool_call_id":"call_2","content":"24C"}]"#
            )),
            Some(with!(
                R,
                r#""model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"weather?"}]},{"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}"},{"type":"function_call","call_id":"call_2","name":"get_weather","arguments":"{\"city\":\"Rome\"}"},{"type":"function_call_output","call_id":"call_1","output":"18C"},{"type":"function_call_output","call_id":"call_2","output":"24C"}]"#
            )),
            Some(with!(
                G,
                r#""contents":[{"role":"user","parts":[{"text":"weather?"}]},{"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}},{"functionCall":{"name":"get_weather","args":{"city":"Rome"}}}]},{"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"result":"18C"}}},{"functionResponse":{"name":"get_weather","response":{"result":"24C"}}}]}]"#
            )),
        ],
        no_vendor: NONE,
        wire: [
            &[
                ("/messages/1/content/1", "Rome"),
                ("/messages/2/content/1", "24C"),
            ],
            &[
                ("/messages/1/tool_calls/1", "Rome"),
                ("/messages/2", "18C"),
                ("/messages/3", "24C"),
            ],
            &[("", "Rome"), ("", "24C")],
            &[
                ("/contents/1/parts/1", "Rome"),
                ("/contents/2/parts/1", "24C"),
            ],
        ],
        wire_only: false,
        check: |r| {
            let calls: Vec<_> = parts(r)
                .filter_map(|p| match p {
                    Part::ToolCall(c) => Some(c),
                    _ => None,
                })
                .collect();
            let results: Vec<_> = parts(r)
                .filter_map(|p| match p {
                    Part::ToolResult(t) => Some(t),
                    _ => None,
                })
                .collect();
            need(
                calls.len() == 2 && results.len() == 2,
                "a call or a result is gone",
            )?;
            for (city, said) in [("Paris", "18C"), ("Rome", "24C")] {
                let call = calls
                    .iter()
                    .find(|c| c.arguments.contains(city))
                    .ok_or("a call's arguments are gone")?;
                let result = results
                    .iter()
                    .find(|t| t.id == call.id)
                    .ok_or("a result answers no call")?;
                need(
                    result.text().contains(said),
                    "the results answer the wrong calls",
                )?;
            }
            Ok(())
        },
    },
    Ask {
        title: "Earlier reasoning handed back beside its tool call (DeepSeek and Kimi refuse the turn without it)",
        samples: [
            Some(with!(
                A,
                r#""model":"m","max_tokens":16000,"thinking":{"type":"enabled","budget_tokens":8000},"messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":[{"type":"thinking","thinking":"I should look it up.","signature":"sig"},{"type":"tool_use","id":"call_1","name":"get_weather","input":{"city":"Paris"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"18C"}]}]"#
            )),
            None,
            Some(with!(
                R,
                r#""model":"m","reasoning":{"effort":"high"},"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"weather?"}]},{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"I should look it up."}]},{"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}"},{"type":"function_call_output","call_id":"call_1","output":"18C"}]"#
            )),
            Some(with!(
                G,
                r#""contents":[{"role":"user","parts":[{"text":"weather?"}]},{"role":"model","parts":[{"text":"I should look it up.","thought":true},{"functionCall":{"name":"get_weather","args":{"city":"Paris"}}}]},{"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"result":"18C"}}}]}]"#
            )),
        ],
        no_vendor: [
            Some("Anthropic takes back only reasoning it signed"),
            None,
            Some("OpenAI takes back only reasoning it sealed"),
            Some("Gemini takes back only thoughts it signed"),
        ],
        wire: [
            &[],
            &[
                ("/messages/1/reasoning_content", "I should look it up."),
                ("/messages/1/tool_calls/0/function/name", "get_weather"),
            ],
            &[],
            &[],
        ],
        wire_only: true,
        check: |r| {
            need(
                parts(r).any(|p| matches!(p, Part::Thinking { .. })),
                "the sample has no reasoning",
            )
        },
    },
    Ask {
        title: "One tool the model must call",
        samples: [
            Some(with!(
                A,
                r#""model":"m","max_tokens":1024,"tool_choice":{"type":"tool","name":"get_weather"},"messages":[{"role":"user","content":"hi"}]"#
            )),
            Some(with!(
                C,
                r#""model":"m","tool_choice":{"type":"function","function":{"name":"get_weather"}},"messages":[{"role":"user","content":"hi"}]"#
            )),
            Some(with!(
                R,
                r#""model":"m","tool_choice":{"type":"function","name":"get_weather"},"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]"#
            )),
            Some(with!(
                G,
                r#""toolConfig":{"functionCallingConfig":{"mode":"ANY","allowedFunctionNames":["get_weather"]}},"contents":[{"role":"user","parts":[{"text":"hi"}]}]"#
            )),
        ],
        no_vendor: NONE,
        wire: [
            &[("/tool_choice/name", "get_weather")],
            &[("/tool_choice/function/name", "get_weather")],
            &[("/tool_choice/name", "get_weather")],
            &[(
                "/toolConfig/functionCallingConfig/allowedFunctionNames/0",
                "get_weather",
            )],
        ],
        wire_only: false,
        check: |r| {
            need(
                r.tool_choice == Some(ToolChoice::Tool("get_weather".into())),
                "the choice is gone",
            )
        },
    },
    Ask {
        title: "An image",
        samples: [
            Some(
                r#"{"model":"m","max_tokens":1024,"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}},{"type":"text","text":"what is this"}]}]}"#,
            ),
            Some(
                r#"{"model":"m","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,iVBORw0KGgo="}},{"type":"text","text":"what is this"}]}]}"#,
            ),
            Some(
                r#"{"model":"m","input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgo="},{"type":"input_text","text":"what is this"}]}]}"#,
            ),
            Some(
                r#"{"contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"image/png","data":"iVBORw0KGgo="}},{"text":"what is this"}]}]}"#,
            ),
        ],
        no_vendor: NONE,
        wire: [
            &[
                ("/messages/0/content/0/source/data", "iVBORw0KGgo="),
                ("/messages/0/content/0/source/media_type", "image/png"),
            ],
            &[(
                "/messages/0/content/0/image_url/url",
                "data:image/png;base64,iVBORw0KGgo=",
            )],
            &[(
                "/input/0/content/0/image_url",
                "data:image/png;base64,iVBORw0KGgo=",
            )],
            &[
                ("/contents/0/parts/0/inlineData/data", "iVBORw0KGgo="),
                ("/contents/0/parts/0/inlineData/mimeType", "image/png"),
            ],
        ],
        wire_only: false,
        check: |r| {
            need(
                parts(r).any(|p| matches!(p, Part::Image(Image::Base64 { media_type, data }) if media_type == "image/png" && data == "iVBORw0KGgo=")),
                "the image is gone",
            )
        },
    },
    Ask {
        title: "Reasoning asked for",
        samples: [
            Some(
                r#"{"model":"m","max_tokens":16000,"thinking":{"type":"enabled","budget_tokens":8000},"messages":[{"role":"user","content":"hi"}]}"#,
            ),
            Some(
                r#"{"model":"m","reasoning_effort":"high","messages":[{"role":"user","content":"hi"}]}"#,
            ),
            Some(
                r#"{"model":"m","reasoning":{"effort":"high"},"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]}"#,
            ),
            Some(
                r#"{"generationConfig":{"thinkingConfig":{"thinkingBudget":8000,"includeThoughts":true}},"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#,
            ),
        ],
        no_vendor: NONE,
        wire: [
            &[("/thinking", "budget_tokens")],
            &[("/reasoning_effort", "")],
            &[("/reasoning/effort", "")],
            &[("/generationConfig/thinkingConfig", "")],
        ],
        wire_only: false,
        check: |r| {
            need(
                r.reasoning.as_ref().is_some_and(|x| !x.disabled()),
                "reasoning is no longer asked for",
            )
        },
    },
    Ask {
        title: "The longest answer and the temperature",
        samples: [
            Some(
                r#"{"model":"m","max_tokens":256,"temperature":0.2,"messages":[{"role":"user","content":"hi"}]}"#,
            ),
            Some(
                r#"{"model":"m","max_tokens":256,"temperature":0.2,"messages":[{"role":"user","content":"hi"}]}"#,
            ),
            Some(
                r#"{"model":"m","max_output_tokens":256,"temperature":0.2,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]}"#,
            ),
            Some(
                r#"{"generationConfig":{"maxOutputTokens":256,"temperature":0.2},"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#,
            ),
        ],
        no_vendor: NONE,
        wire: [
            &[("/max_tokens", "256"), ("/temperature", "0.2")],
            &[("/max_completion_tokens", "256"), ("/temperature", "0.2")],
            &[("/max_output_tokens", "256"), ("/temperature", "0.2")],
            &[
                ("/generationConfig/maxOutputTokens", "256"),
                ("/generationConfig/temperature", "0.2"),
            ],
        ],
        wire_only: false,
        check: |r| {
            need(r.max_tokens == Some(256), "the longest answer changed")?;
            need(r.temperature == Some(0.2), "the temperature changed")
        },
    },
    Ask {
        title: "Words that end the answer",
        samples: [
            Some(
                r#"{"model":"m","max_tokens":1024,"stop_sequences":["END"],"messages":[{"role":"user","content":"hi"}]}"#,
            ),
            Some(r#"{"model":"m","stop":["END"],"messages":[{"role":"user","content":"hi"}]}"#),
            None,
            Some(
                r#"{"generationConfig":{"stopSequences":["END"]},"contents":[{"role":"user","parts":[{"text":"hi"}]}]}"#,
            ),
        ],
        no_vendor: [None, None, Some("the Responses API has none"), None],
        wire: [
            &[("/stop_sequences/0", "END")],
            &[("/stop/0", "END")],
            &[],
            &[("/generationConfig/stopSequences/0", "END")],
        ],
        wire_only: false,
        check: |r| need(r.stop == ["END"], "the words are gone"),
    },
    Ask {
        title: "Another vendor's seal on earlier reasoning is kept from this one",
        samples: [
            Some(with!(
                A,
                r#""model":"m","max_tokens":16000,"thinking":{"type":"enabled","budget_tokens":8000},"messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":[{"type":"thinking","thinking":"I should look it up.","signature":"SEAL-OF-ANTHROPIC"},{"type":"tool_use","id":"call_1","name":"get_weather","input":{"city":"Paris"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"18C"}]}]"#
            )),
            None,
            Some(with!(
                R,
                r#""model":"m","reasoning":{"effort":"high"},"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"weather?"}]},{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"I should look it up."}],"encrypted_content":"SEAL-OF-OPENAI"},{"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Paris\"}"},{"type":"function_call_output","call_id":"call_1","output":"18C"}]"#
            )),
            Some(with!(
                G,
                r#""contents":[{"role":"user","parts":[{"text":"weather?"}]},{"role":"model","parts":[{"functionCall":{"name":"get_weather","args":{"city":"Paris"}},"thoughtSignature":"SEAL-OF-GEMINI"}]},{"role":"user","parts":[{"functionResponse":{"name":"get_weather","response":{"result":"18C"}}}]}]"#
            )),
        ],
        no_vendor: NONE,
        wire: [
            &[("", "!SEAL-OF"), ("", "18C")],
            &[("", "!SEAL-OF"), ("", "18C")],
            &[("", "!SEAL-OF"), ("", "18C")],
            &[("", "!SEAL-OF"), ("", "18C")],
        ],
        wire_only: true,
        check: |r| {
            need(
                parts(r).any(|p| matches!(p, Part::ToolCall(_))),
                "the sample has no call",
            )
        },
    },
    Ask {
        title: "Codex's free-form tool (`apply_patch`)",
        samples: [
            None,
            None,
            Some(
                r#"{"model":"m","tools":[{"type":"custom","name":"apply_patch","description":"Patch files"}],"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"fix it"}]},{"type":"custom_tool_call","call_id":"call_1","name":"apply_patch","input":"*** Begin Patch"},{"type":"custom_tool_call_output","call_id":"call_1","output":"Done"}]}"#,
            ),
            None,
        ],
        no_vendor: NONE,
        wire: [
            &[
                ("/tools/0/name", "apply_patch"),
                ("/tools/0/input_schema", "input"),
                ("/messages/1", "*** Begin Patch"),
                ("/messages/2", "Done"),
            ],
            &[
                ("/tools/0/function/name", "apply_patch"),
                ("/tools/0/function/parameters", "input"),
                ("/messages/1", "*** Begin Patch"),
                ("/messages/2", "Done"),
            ],
            &[],
            &[
                ("/tools/0/functionDeclarations/0/name", "apply_patch"),
                ("/tools/0/functionDeclarations/0", "input"),
                ("/contents/1", "*** Begin Patch"),
                ("/contents/2", "Done"),
            ],
        ],
        wire_only: true,
        check: |r| {
            need(
                r.tools.first().is_some_and(|t| t.custom),
                "the tool is no longer free-form",
            )
        },
    },
];
