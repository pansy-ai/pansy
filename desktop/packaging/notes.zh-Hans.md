## pansy 0.1.1

- 新增 49 个智能体：Pi、Goose、Copilot CLI、VS Code、Droid、Kimi Code、Qoder、Hermes、OpenClaw 等，都是原地接入，断开后恢复原样。
- 智能体页：显示智能体的版本并可更新，拖动调整顺序，Claude Code 的模型档位与推理强度，Codex 的推理强度，以及智能体列出哪些模型。
- 故障切换：被某个模型拒绝的请求会交给路由组里的下一个模型；限流只让 Key 在触发它的模型上暂停，不影响它的其他模型；连接中断时先重试一次。
- 导入：提供 CC Switch、Claude Code、Codex 和 OpenCode 里已有的供应商。
- 首次读取智能体的会话时，会话页显示 pansy 的标志。
