# ChatRequestBuilder 修复：tool_calls/tool response 消息顺序与 ID 匹配

> 设计日期：2026-07-23  
> 状态：已实施

---

## 一、问题

使用 DeepSeek V4 Flash（Chat Completions API）时，API 返回错误：

```
{"error":{"message":"An assistant message with 'tool_calls' must be followed by tool messages
responding to each 'tool_call_id'. (insufficient tool messages following tool_calls message)"}}
```

## 二、根因分析

### 2.1 空 assistant 消息插入在 tool_calls 和 tool response 之间

DeepSeek 在流式响应中发送 `delta.content: ""`（空字符串），`process_chat_sse()` 的 `append_assistant_text` 无条件创建了 `Message(role=assistant, content=[OutputText{""}])`。在 ChatRequestBuilder 中，该空消息被处理为 `{"role": "assistant", "content": ""}`，出现在 `tool_calls` 消息和 `tool` 消息之间，导致 API 校验失败。

### 2.2 有内容的 assistant 文本插入在 tool_calls 和 tool response 之间

当模型在工具调用后迅速生成文本回复时，历史中的存储顺序可能是：

```
FunctionCall(call_1)
Message(assistant, "文本回复")    ← 模型文本
FunctionCallOutput(call_1)        ← 工具结果
```

ChatRequestBuilder 处理为：

```
assistant(tool_calls=[call_1])
assistant("文本回复")              ← 插在中间！
tool(call_1)
```

违反了 Chat API "tool_calls 必须紧跟在 tool response 之前" 的格式要求。

### 2.3 CustomToolCall / LocalShellCall 使用了错误的 ID 字段

`CustomToolCall` 和 `LocalShellCall` 使用 `id` 字段（`Option<ResponseItemId>`）作为 tool_call 的标识符，但对应的 tool response（`CustomToolCallOutput` / `FunctionCallOutput`）使用 `call_id` 字段（`String`）。两值不同，导致 API 认为 tool_call 没有对应的 tool response。

## 三、修复方案

### 3.1 合并/跳过 tool_calls 后的 assistant 消息

在 `ChatRequestBuilder::build()` 的 `Message(assistant)` 处理器中增加检查：

```rust
if role == "assistant"
    && messages.last().is_some_and(|m| m.get("tool_calls").is_some())
{
    if text.trim().is_empty() {
        continue;     // 空内容→跳过
    }
    // 有内容→合并到 tool_calls 消息
    obj.insert("content".to_string(), json!(text));
    continue;
}
```

### 3.2 修复 CustomToolCall 和 LocalShellCall 的 ID 字段

| 类型 | 修改前 | 修改后 |
|------|--------|--------|
| `LocalShellCall` | `"id": id.clone().map(\|i\| i.to_string()).unwrap_or_default()` | `"id": call_id.clone().unwrap_or_default()` |
| `CustomToolCall` | `"id": id` | `"id": call_id` |

同时将非标准的 `"type": "custom"` 和 `"type": "local_shell_call"` 统一为 Chat API 标准格式 `"type": "function"`。

## 四、修改文件

- `codex-rs/codex-api/src/requests/chat.rs`：ChatRequestBuilder 修复

## 五、验证

- 单元测试 `requests::chat::tests` 全部通过
- DeepSeek V4 Flash 端到端测试通过（多次工具调用后不再报错）
