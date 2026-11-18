# What the gateway carries between protocols

Written by `cargo test -p pansy-gateway --test compat`, which fails when this page
and the code disagree. A request in the vendor's own protocol passes through
untouched and is not in these tables.

A column is the protocol the **vendor** speaks. ✓: the thing arrives from each of
the other three protocols. —: the protocol has no such thing, and why. ✗: it is
lost, from where, and how.

## Requests

| | Anthropic | Chat | Responses | Gemini |
| :-- | :-- | :-- | :-- | :-- |
| System instructions | ✓ | ✓ | ✓ | ✓ |
| A conversation of several turns | ✓ | ✓ | ✓ | ✓ |
| Tools the model may call | ✓ | ✓ | ✓ | ✓ |
| A tool call and its result, earlier in the conversation | ✓ | ✓ | ✓ | ✓ |
| Two tool calls in one turn, and their results | ✓ | ✓ | ✓ | ✓ |
| Earlier reasoning handed back beside its tool call (DeepSeek and Kimi refuse the turn without it) (asked in Anthropic, Responses, Gemini) | — Anthropic takes back only reasoning it signed | ✓ | — OpenAI takes back only reasoning it sealed | — Gemini takes back only thoughts it signed |
| One tool the model must call | ✓ | ✓ | ✓ | ✓ |
| An image | ✓ | ✓ | ✓ | ✓ |
| Reasoning asked for | ✓ | ✓ | ✓ | ✓ |
| The longest answer and the temperature | ✓ | ✓ | ✓ | ✓ |
| Words that end the answer (asked in Anthropic, Chat, Gemini) | ✓ | ✓ | — the Responses API has none | ✓ |
| Another vendor's seal on earlier reasoning is kept from this one (asked in Anthropic, Responses, Gemini) | ✓ | ✓ | ✓ | ✓ |
| Codex's free-form tool (`apply_patch`) (asked in Responses) | ✓ | ✓ | ✓ | ✓ |

The Chat column is a vendor marked as OpenAI's kind: only those are sent
`reasoning_effort`, `max_completion_tokens` and reasoning back as `reasoning_content`.

## Answers

A column is the protocol the **agent** speaks; the answer comes from a vendor of each
of the other three.

### Whole

| | Anthropic | Chat | Responses | Gemini |
| :-- | :-- | :-- | :-- | :-- |
| Text, and what it cost | ✓ | ✓ | ✓ | ✓ |
| A tool call | ✓ | ✓ | ✓ | ✓ |
| Reasoning, then text | ✓ | ✓ | ✓ | ✓ |
| An answer cut off at its longest | ✓ | ✓ | ✓ | ✓ |

### Streamed

| | Anthropic | Chat | Responses | Gemini |
| :-- | :-- | :-- | :-- | :-- |
| Text, and what it cost | ✓ | ✓ | ✓ | ✓ |
| A tool call | ✓ | ✓ | ✓ | ✓ |
| Reasoning, then text | ✓ | ✓ | ✓ | ✓ |
| An answer cut off at its longest | ✓ | ✓ | ✓ | ✓ |

