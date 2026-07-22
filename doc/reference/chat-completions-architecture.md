# Chat Completions API 架构

> 设计日期：2026-07-23  
> 状态：已实施（第一版）

---

## 一、背景

Codex 原生使用 OpenAI **Responses API**（`/v1/responses`）。为支持 DeepSeek V4 等第三方提供商，引入了 **Chat Completions API**（`/v1/chat/completions`）兼容层。

两套 API 在消息格式、工具调用、流式传输等方面存在差异，因此在代码中需要两套并行的处理路径。

---

## 二、整体架构

```
TUI / CLI
    │
    ▼
core/src/client.rs ── ModelClientSession::stream()
    │
    ├── WireApi::Responses ──→ stream_responses_api()
    │       │
    │       ▼
    │   codex-api 原生 Responses 格式
    │   {"input": [...], "tools": [...]}
    │
    └── WireApi::Chat ──→ stream_chat_api()
            │
            ▼
        codex-api/src/requests/chat.rs
        ChatRequestBuilder::build()
        {"messages": [...], "tools": [...]}
```

### 2.1 分发逻辑

`ModelClientSession::stream()` 根据 `Provider::wire` 字段（`WireApi` 枚举）决定走哪条路径：

- `WireApi::Responses`（默认）→ `stream_responses_api()`
- `WireApi::Chat` → `stream_chat_api()`

### 2.2 Provider 配置

Provider 在 `config.toml` 中配置 `wire_api` 字段：

```toml
[model_providers.deepseek]
wire_api = "chat"          # 使用 Chat Completions API
base_url = "https://api.deepseek.com"
```

---

## 三、Chat Completions 路径详解

### 3.1 构建请求

`ChatRequestBuilder`（`codex-api/src/requests/chat.rs`）将 Codex 内部的 `Vec<ResponseItem>` 历史转换为 Chat API 的 `messages` 数组。

关键映射：

| ResponseItem 类型 | Chat API 消息 |
|------------------|---------------|
| `Message(role="user")` | `{"role": "user", "content": "..."}` |
| `Message(role="assistant")` | `{"role": "assistant", "content": "..."}` |
| `Message(role="developer")` | `{"role": "system", "content": "..."}` |
| `FunctionCall` | 合并到前一条 `assistant` 消息的 `tool_calls` |
| `FunctionCallOutput` | `{"role": "tool", "tool_call_id": "...", "content": "..."}` |
| `CustomToolCall` | 转换为标准 `function` 类型 tool_call |
| `LocalShellCall` | 转换为标准 `function` 类型 tool_call |

### 3.2 SSE 响应解析

`process_chat_sse()`（`codex-api/src/sse/chat.rs`）解析 Chat API 的 SSE 流，产生 `ResponseEvent` 事件（与 Responses API 路径兼容的事件格式）。

关键处理：
- `delta.content`：追加到当前 assistant 消息
- `delta.tool_calls`：累积工具调用参数
- `finish_reason: "tool_calls"`：刷新 assistant 消息，然后发射 `FunctionCall` 事件

### 3.3 工具调用格式规范

Chat API 要求 `tool_calls` 必须紧跟在对应的 tool response 之前，中间不能插入其他消息。消息序列必须严格遵循：

```
assistant(content=null/string, tool_calls=[...])  ← 模型发起工具调用
tool(tool_call_id=...)                             ← 工具执行结果
assistant(content=string, tool_calls=[...])         ← 模型继续
tool(tool_call_id=...)
assistant(content=string)                          ← 模型最终回复
```

---

## 四、常见问题与处理

### 4.1 历史中 FunctionCall 排在 Message 之前

在 Chat API SSE 解析中，工具调用完成时先发射 assistant 消息、再发射 FunctionCall。但某些情况下历史存储的顺序可能相反（FunctionCall 在前，Message 在后）。

`ChatRequestBuilder` 中的处理：

1. 当 `Message(assistant)` 的内容为空且上一条已发出的消息有 `tool_calls` 时，**跳过该空消息**
2. 当 `Message(assistant)` 有实际文本内容且上一条已发出的消息有 `tool_calls` 时，**将文本合并到 tool_calls 消息的 `content` 字段中**

### 4.2 CustomToolCall / LocalShellCall 的 ID 字段

Chat API 的 tool_call `id` 必须与 tool response 的 `tool_call_id` 完全匹配。`CustomToolCall` 和 `LocalShellCall` 必须使用 `call_id` 字段（而非 `id` 字段）作为 tool_call 的标识符。

---

## 五、相关文件

| 文件 | 用途 |
|------|------|
| `codex-api/src/requests/chat.rs` | ChatRequestBuilder：构建 Chat API 请求体 |
| `codex-api/src/endpoint/chat.rs` | ApiChatClient：发送 HTTP 请求 |
| `codex-api/src/sse/chat.rs` | 解析 Chat API SSE 流 |
| `core/src/client.rs` | 分发 `WireApi::Chat` 到 `stream_chat_api` |
| `codex-api/src/provider.rs` | `WireApi` 枚举定义 |
| `model-provider-info/src/lib.rs` | Provider 配置到 `ApiWireApi` 的转换 |
| `tools/src/tool_spec.rs` | `create_tools_json_for_chat_completions_api()` |
