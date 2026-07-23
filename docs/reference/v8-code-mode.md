# V8 Code Mode (exec / wait)

> 分析日期：2026-07-23
> 状态：已完整实现，需配置启用（Feature::CodeMode，Stage::UnderDevelopment）

---

## 一、概述

Codex 通过集成 Google V8 JavaScript 引擎，为 AI 大模型提供了 **Code Mode** 能力——允许模型编写 JavaScript 脚本来编排复杂工具调用，支持并行执行多个工具、条件控制流、数据传递等高级编排场景。

核心工具：

| 工具名 | 用途 |
|--------|------|
| `exec`  | 在 V8 isolate 中执行 JavaScript 脚本，可调用所有嵌套工具 |
| `wait`  | 跟踪长时间运行的 exec 脚本 cell，获取后续输出或终止 |

---

## 二、已知限制：Chat Completions API 不支持 Freeform 工具

### 问题描述

`exec` 工具在 `ToolSpec` 中定义为 `Freeform` 类型：

```rust
// codex-rs/tools/src/tool_spec.rs
pub enum ToolSpec {
    #[serde(rename = "function")]
    Function(ResponsesApiTool),
    #[serde(rename = "custom")]
    Freeform(FreeformTool),      // ← exec 工具走这条路
    ...
}
```

Chat Completions API 的工具构建函数只接受 `type="function"` 的工具：

```rust
// codex-rs/tools/src/tool_spec.rs:93
pub fn create_tools_json_for_chat_completions_api(tools: &[ToolSpec]) -> ... {
    .filter_map(|mut tool| {
        if tool.get("type") != Some(&Value::String("function".to_string())) {
            return None;  // ← exec (type="custom") 被丢弃
        }
        ...
    })
}
```

### 影响范围

| wire_api | exec 工具是否可用 | 使用模型示例 |
|----------|-------------------|-------------|
| `responses` | ✅ 可用 | OpenAI GPT 系列 |
| `chat` | ❌ 被过滤 | DeepSeek、Mimo 等第三方模型 |

### 判定逻辑链路

```
config.toml: model_provider = "xxx"
  → provider 定义: wire_api = "chat" 或 "responses"
    → client.rs 分支:
      ├── wire_api=responses → create_tools_json_for_responses_api() → exec 保留 ✅
      └── wire_api=chat      → create_tools_json_for_chat_completions_api() → exec 过滤 ❌
```

**结论：当前 Code Mode 仅在 Responses API 路径下完整可用。**

---

## 三、架构总览

```
用户请求
    │
    ▼
TUI / CLI
    │
    ▼
core/src/client.rs ── wire_api 路由
    │
    ├── responses ──→ create_tools_json_for_responses_api()
    │                   → exec (type=custom) 保留 ✅
    │
    └── chat ──────→ create_tools_json_for_chat_completions_api()
                      → exec (type=custom) 被过滤 ❌
    │
    ▼
core/src/tools/code_mode/
    ├── execute_spec.rs      ← 构建 exec 工具定义（含所有嵌套工具描述）
    ├── execute_handler.rs   ← 处理 exec 调用 → CodeModeService.execute()
    ├── wait_handler.rs      ← 处理 wait 调用 → CodeModeService.wait()
    ├── delegate.rs          ← 嵌套工具调度（CodeModeDispatchBroker）
    └── mod.rs               ← CodeModeService 主结构
            │
            ▼
    CodeModeSessionProvider
    ├── InProcessCodeModeSessionProvider   ← V8 在 codex 进程内运行
    └── ProcessOwnedCodeModeSessionProvider ← V8 在独立子进程运行
            │
            ▼
    code-mode crate (codex-rs/code-mode/)
    ├── service.rs           ← InProcessCodeModeSession 实现
    ├── session_runtime/     ← 会话生命周期管理
    ├── runtime/
    │   ├── globals.rs       ← JS 全局对象（tools, text, image, store...）
    │   ├── callbacks.rs     ← V8 ↔ Rust 回调桥接
    │   ├── module_loader.rs ← ES 模块加载
    │   └── timers.js        ← setTimeout/clearTimeout
    ├── cell_actor/          ← 单个 exec cell 的状态机
    └── remote_session/      ← 子进程模式的远程会话
            │
            ▼
    code-mode-protocol crate (codex-rs/code-mode-protocol/)
    ├── description.rs       ← exec/wait 工具描述模板
    ├── runtime.rs           ← ExecuteRequest, WaitRequest, RuntimeResponse 等
    ├── session.rs           ← CodeModeSession trait
    └── host/                ← 子进程通信协议（codec/message/payload）
```

---

## 四、exec 工具详细说明

### 4.1 调用方式

exec 接受原始 JavaScript 源码（非 JSON、非 markdown 代码块）：

```
// 可选首行 pragma
// @exec: {"yield_time_ms": 10000, "max_output_tokens": 1000}

// JavaScript 代码
const result = await tools.exec_command("ls -la");
text(result);
```

### 4.2 嵌套工具调用

所有 codex 工具通过 `tools` 全局对象暴露为 JavaScript 函数：

```javascript
// 串行调用
const list = await tools.exec_command("ls");
const content = await tools.exec_command("cat file.txt");

// 并行调用（Promise.all）
const [a, b] = await Promise.all([
    tools.exec_command("task1"),
    tools.exec_command("task2"),
]);
```

工具名自动转换为合法 JS 标识符，例如 `mcp__ologs__get_profile`。

### 4.3 全局辅助函数

| 函数 | 说明 |
|------|------|
| `text(value)` | 输出文本项 |
| `image(url, detail?)` | 输出图片（base64 data URL） |
| `audio(url)` | 输出音频（base64 data URL） |
| `generatedImage(result)` | 输出图片生成结果 |
| `store(key, value)` | 跨 exec 调用的键值存储 |
| `load(key)` | 读取存储的值 |
| `notify(value)` | 立即注入额外输出 |
| `setTimeout(cb, ms?)` | 定时器 |
| `clearTimeout(id)` | 取消定时器 |
| `yield_control()` | 立即输出当前累积结果，脚本继续运行 |
| `exit()` | 立即结束脚本（类似顶层 early return） |
| `ALL_TOOLS` | 元数据数组 `{ name, description }` |

### 4.4 输出截断

- 默认 `max_output_tokens`: 10000
- 支持 pragma 覆盖：`// @exec: {"max_output_tokens": 5000}`
- 超限时自动截断并附加警告

---

## 五、wait 工具详细说明

当 exec 返回 `"Script running with cell ID ..."` 时，使用 wait 获取后续输出：

```json
{
    "cell_id": "1",
    "yield_time_ms": 10000,
    "max_tokens": 10000,
    "terminate": false
}
```

| 参数 | 说明 |
|------|------|
| `cell_id` | exec 返回的 cell 标识 |
| `yield_time_ms` | 等待新输出的超时时间 |
| `max_tokens` | 本次 wait 返回的最大 token 数 |
| `terminate` | `true` 则终止该 cell |

- cell 仍在运行 → 返回 `Yielded` + cell_id，可继续 wait
- cell 已完成 → 返回 `Result` 或 `Terminated`
- cell 不存在 → 返回 `MissingCell`

---

## 六、配置启用

### 6.1 config.toml 配置

```toml
[features.code_mode]
enabled = true
```

### 6.2 相关 Feature Flags

| Feature | Key | 默认值 | 说明 |
|---------|-----|--------|------|
| `CodeMode` | `code_mode` | `false` | 启用 V8 exec/wait |
| `CodeModeHost` | `code_mode_host` | `false` | 使用独立子进程运行 V8（隔离性更好） |
| `CodeModeOnly` | `code_mode_only` | `false` | 仅暴露 exec/wait，隐藏其他工具 |
| `CodeModeBufferedExec` | `code_mode_buffered_exec` | `false` | exec 默认 yield_time 改为 30s |

### 6.3 排除特定工具

```toml
[features.code_mode]
enabled = true
excluded_tool_namespaces = ["mcp__dangerous"]
direct_only_tool_namespaces = ["mcp__admin"]
```

---

## 七、运行时模式

### 7.1 In-Process 模式（默认）

V8 isolate 直接在 codex 进程内创建和销毁。

- 优点：无进程间通信开销，延迟低
- 缺点：V8 崩溃会影响主进程

### 7.2 Host Process 模式（CodeModeHost）

V8 运行在独立的 `codex-code-mode-host` 子进程中。

- 优点：隔离性好，V8 崩溃不影响主进程
- 缺点：进程间通信开销
- 若 host 程序不存在，自动 fallback 到 In-Process 模式

---

## 八、关键代码路径

| 文件 | 职责 |
|------|------|
| `codex-rs/features/src/lib.rs` | Feature 定义与启用逻辑 |
| `codex-rs/core/src/tools/mod.rs` | `effective_tool_mode()` 决定工具模式 |
| `codex-rs/core/src/tools/spec_plan.rs` | 根据 tool_mode 注入 exec/wait 工具 |
| `codex-rs/core/src/tools/code_mode/mod.rs` | CodeModeService 主结构 |
| `codex-rs/core/src/tools/code_mode/execute_handler.rs` | exec 工具处理器 |
| `codex-rs/core/src/tools/code_mode/wait_handler.rs` | wait 工具处理器 |
| `codex-rs/core/src/tools/code_mode/delegate.rs` | 嵌套工具调度 Broker |
| `codex-rs/core/src/session/session.rs` | Session 初始化中创建 CodeModeService |
| `codex-rs/core/src/thread_manager.rs` | 选择 SessionProvider |
| `codex-rs/core/src/client.rs` | wire_api 路由，决定工具序列化路径 |
| `codex-rs/tools/src/tool_spec.rs` | `create_tools_json_for_chat_completions_api()` 过滤逻辑 |
| `codex-rs/code-mode/src/service.rs` | InProcessCodeModeSession 实现 |
| `codex-rs/code-mode/src/runtime/globals.rs` | JS 全局对象注册 |
| `codex-rs/code-mode/src/runtime/callbacks.rs` | V8 ↔ Rust 回调 |
| `codex-rs/code-mode-protocol/src/description.rs` | exec/wait 工具描述模板 |
| `codex-rs/code-mode-protocol/src/runtime.rs` | 请求/响应协议类型 |

---

## 九、工具模式判定逻辑

```rust
fn effective_tool_mode(turn_context: &TurnContext) -> ToolMode {
    turn_context.model_info.tool_mode.unwrap_or_else(|| {
        if features.enabled(Feature::CodeModeOnly) {
            ToolMode::CodeModeOnly
        } else if features.enabled(Feature::CodeMode) {
            ToolMode::CodeMode
        } else {
            ToolMode::Direct  // 当前默认：直接工具调用
        }
    })
}
```

| 模式 | 暴露的工具 |
|------|-----------|
| `Direct` | 所有工具直接暴露（当前默认） |
| `CodeMode` | exec + wait + 所有工具（exec 内可调用） |
| `CodeModeOnly` | 仅 exec + wait（工具只能在脚本内调用） |

---

## 十、结论

Code Mode 是一个**已完整实现**的功能，具备：

- ✅ 完整的协议定义（code-mode-protocol）
- ✅ 完整的 V8 运行时（code-mode crate）
- ✅ 完整的核心集成（core/src/tools/code_mode/）
- ✅ 两种运行模式（In-Process / Host Process）
- ✅ 测试覆盖（单元测试 + 集成测试）
- ✅ 无 TODO/FIXME/未实现标记
- ✅ 无条件编译限制（`#[cfg]`）

**当前限制**：仅在 `wire_api = "responses"` 的模型上可用。使用 `wire_api = "chat"` 的第三方模型（如 DeepSeek、Mimo）因工具类型不兼容而无法使用。

**启用方式**：在 `~/.codex/config.toml` 中添加 `[features.code_mode] enabled = true`，并确保使用支持 Responses API 的模型。
