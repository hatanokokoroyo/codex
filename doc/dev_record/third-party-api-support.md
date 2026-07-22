# 第三方 OpenAI 兼容 API 支持开发参考

> 研究日期：2026-07-22  
> 研究目标：为 CLI 添加对 DeepSeek 等第三方 OpenAI 兼容 API 的支持  
> 状态：架构分析完成，待实施

---

## 一、需求概述

当前 CLI 启动时显示三个登录选项（Sign in with ChatGPT / Sign in with Device Code / Provide your own API key），全部针对 OpenAI 服务。需要额外增加对第三方 OpenAI API 格式服务的支持（如 DeepSeek V4 Flash/Pro）。

### 关键约束

1. DeepSeek 等第三方服务使用 **Chat Completions API**（`/v1/chat/completions`），而非 Responses API
2. 项目当前**只支持 Responses API**，Chat Completions 支持已被主动移除
3. 移除提交：`d2394a249`（`chore: nuke chat/completions API #10157`，2026-02-03）

---

## 二、项目架构分析

### 2.1 启动与认证流程

#### TUI Onboarding 登录选项

**文件**：`codex-rs/tui/src/onboarding/auth.rs`

三个选项定义在 `SignInOption` 枚举（line 90-94）：
```rust
pub(crate) enum SignInOption {
    ChatGpt,
    DeviceCode,
    ApiKey,
}
```

状态机 `SignInState`（line 78-87）管理登录流程：
- `PickMode` → 用户选择登录方式
- `ChatGptContinueInBrowser` → OAuth 浏览器登录
- `ChatGptDeviceCode` → OAuth 设备码登录
- `ApiKeyEntry` → 手动输入 API Key
- `ChatGptSuccess` / `ApiKeyConfigured` → 登录成功

#### API Key 登录流程

1. 用户选择 "Provide your own API key" → `start_api_key_entry()`（line 770-798）
2. 读取 `OPENAI_API_KEY` 环境变量预填充（line 776）
3. 用户确认后调用 `save_api_key()`（line 800-844）
4. 发送 `ClientRequest::LoginAccount` 到 app-server
5. 服务端调用 `login_with_api_key()` 写入 `auth.json`

**持久化位置**：`~/.codex/auth.json`（或 keyring，取决于 `AuthCredentialsStoreMode`）

#### 登录屏幕显示控制

**文件**：`codex-rs/tui/src/lib.rs`

```rust
// line 1410-1417
let login_status = if initial_config.model_provider.requires_openai_auth {
    get_login_status(app_server, &initial_config).await?
} else {
    LoginStatus::NotAuthenticated  // 跳过登录
};
```

```rust
// line 2009-2017 should_show_login_screen()
if !config.model_provider.requires_openai_auth {
    return false;  // 不显示登录屏幕
}
```

**关键逻辑**：当 `requires_openai_auth = false` 时，整个登录流程被跳过。

#### `forced_login_method` 配置

**文件**：`codex-rs/config/src/config_toml.rs:247`

可选值：`Chatgpt` 或 `Api`

- `Chatgpt`：只显示 ChatGPT 相关选项，禁止 API Key 登录
- `Api`：只显示 API Key 登录，禁止 ChatGPT 选项

**实现位置**：`auth.rs:307-336`（`is_api_login_allowed()` / `is_chatgpt_login_allowed()`）

---

### 2.2 模型提供商系统

#### ModelProviderInfo 结构体

**文件**：`codex-rs/model-provider-info/src/lib.rs:89-141`

```rust
pub struct ModelProviderInfo {
    pub name: String,                          // 显示名称
    pub base_url: Option<String>,              // API 基础 URL
    pub env_key: Option<String>,               // API Key 环境变量名
    pub env_key_instructions: Option<String>,  // 环境变量说明
    pub experimental_bearer_token: Option<String>,
    pub auth: Option<ModelProviderAuthInfo>,    // 命令获取 token
    pub aws: Option<ModelProviderAwsAuthInfo>,  // AWS SigV4 认证
    pub wire_api: WireApi,                     // 协议类型（当前只有 Responses）
    pub query_params: Option<HashMap<String, String>>,
    pub http_headers: Option<HashMap<String, String>>,
    pub env_http_headers: Option<HashMap<String, String>>,
    pub request_max_retries: Option<u64>,
    pub stream_max_retries: Option<u64>,
    pub stream_idle_timeout_ms: Option<u64>,
    pub websocket_connect_timeout_ms: Option<u64>,
    pub requires_openai_auth: bool,            // 是否需要 OpenAI 认证
    pub supports_websockets: bool,
}
```

#### WireApi 枚举（当前状态）

**文件**：`codex-rs/model-provider-info/src/lib.rs:57-84`

```rust
pub enum WireApi {
    #[default]
    Responses,  // 唯一变体
}
```

`Chat` 变体已被移除，反序列化 `"chat"` 会报错：
```
`wire_api = "chat"` is no longer supported.
How to fix: set `wire_api = "responses"` in your provider config.
```

#### 内置提供商列表

**文件**：`codex-rs/model-provider-info/src/lib.rs:433-459`（`built_in_model_providers()`）

| ID | 名称 | base_url | requires_openai_auth |
|----|------|----------|---------------------|
| `openai` | OpenAI | `https://chatgpt.com/backend-api/codex` 或 `https://api.openai.com/v1` | `true` |
| `amazon-bedrock` | Amazon Bedrock | `https://bedrock-mantle.us-east-1.api.aws/openai/v1` | `false` |
| `ollama` | gpt-oss | `http://localhost:11434/v1` | `false` |
| `lmstudio` | gpt-oss | `http://localhost:1234/v1` | `false` |

#### 配置加载流程

**文件**：`codex-rs/core/src/config/mod.rs:3549-3566`

```rust
let model_providers = merge_configured_model_providers(
    built_in_model_providers(openai_base_url), cfg.model_providers
);
let model_provider_id = model_provider.or(cfg.model_provider)
    .unwrap_or_else(|| "openai".to_string());
let model_provider = model_providers.get(&model_provider_id)...clone();
```

用户配置示例（`~/.codex/config.toml`）：
```toml
model_provider = "deepseek"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
env_key = "DEEPSEEK_API_KEY"
requires_openai_auth = false
```

---

### 2.3 API 请求管线

#### 请求路由

**文件**：`codex-rs/core/src/client.rs`

```rust
// line 1790-1828 (近似)
match wire_api {
    WireApi::Responses => {
        // 调用 stream_responses_api()
        // POST {base_url}/responses
    }
}
```

#### ResponsesClient

**文件**：`codex-rs/codex-api/src/endpoint/responses.rs`

- 端点路径：`"responses"`（line 100-102）
- 方法：`POST`
- Accept：`text/event-stream`
- 请求体：`ResponsesApiRequest` JSON

#### ResponsesApiRequest 结构

**文件**：`codex-rs/codex-api/src/common.rs:216-239`

```rust
pub struct ResponsesApiRequest {
    pub model: String,
    pub instructions: String,
    pub input: Vec<ResponseItem>,        // 不是 messages 数组
    pub tools: Option<Vec<Value>>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
    pub reasoning: Option<Reasoning>,    // Responses API 特有
    pub store: bool,
    pub stream: bool,
    pub stream_options: Option<StreamOptions>,
    pub include: Vec<String>,            // Responses API 特有
    pub service_tier: Option<String>,
    pub prompt_cache_key: Option<String>,
    pub text: Option<TextControls>,      // Responses API 特有
    pub client_metadata: Option<HashMap<String, String>>,
}
```

**这是 Responses API 专用格式**，与 Chat Completions 的 `messages` 数组完全不同。

#### SSE 事件类型

**文件**：`codex-rs/codex-api/src/common.rs:74-121`

```rust
pub enum ResponseEvent {
    Created { response_id: String },
    OutputItemAdded(ResponseItem),
    OutputItemDone(ResponseItem),
    OutputTextDelta(String),
    ReasoningContentDelta { delta: String, content_index: i64 },
    ReasoningSummaryDelta { delta: String, summary_index: i64 },
    ToolCallInputDelta { call_id: String, delta: String },
    Completed { response_id: String, token_usage: Option<TokenUsage> },
    RateLimits { ... },
}
```

---

### 2.4 认证系统

#### CodexAuth 枚举

**文件**：`codex-rs/login/src/auth/manager.rs:73-81`

```rust
pub enum CodexAuth {
    ApiKey(ApiKeyAuth),
    Chatgpt(ChatgptAuth),
    ChatgptAuthTokens(ChatgptAuthTokens),
    Headers(AuthHeaders),
    AgentIdentity(AgentIdentityAuth),
    PersonalAccessToken(PersonalAccessTokenAuth),
    BedrockApiKey(BedrockApiKeyAuth),
}
```

#### 认证解析流程

**文件**：`codex-rs/model-provider/src/auth.rs:179-197`（`resolve_provider_auth()`）

优先级：
1. `ModelProviderInfo.api_key()`（从 `env_key` 环境变量读取）→ `BearerAuthProvider`
2. `CodexAuth`（从 `auth.json` 加载）→ `auth_provider_from_auth()`
3. 无认证 → `unauthenticated_auth_provider()`

#### 首方认证路径判断

**文件**：`codex-rs/model-provider/src/provider.rs:223-229`

`provider_uses_first_party_auth_path()` 返回 `true` 的条件：
- `requires_openai_auth == true` **且**
- 无外部认证字段（`env_key`、`bearer_token`、`auth`、`aws`）

---

## 三、历史代码分析（Chat Completions 支持）

### 3.1 移除提交

**Commit**：`d2394a249`（`chore: nuke chat/completions API #10157`）  
**日期**：2026-02-03  
**作者**：jif-oai  
**变更**：49 files changed, 268 insertions(+), 2931 deletions(-)

### 3.2 被删除的核心文件

#### `codex-api/src/requests/chat.rs`（494 行）

Chat Completions 请求构建器。核心逻辑：

- 将 `ResponseItem[]`（内部格式）转换为 `messages[]`（Chat Completions 格式）
- 处理 `system` → `user` → `assistant` → `tool` 消息角色映射
- 合并连续的 tool_calls 到单个 assistant 消息
- 处理 reasoning 内容的锚定和附加

关键方法：`ChatRequestBuilder::build()` → 生成 `ChatRequest { body: Value, headers: HeaderMap }`

生成的请求体格式：
```json
{
  "model": "...",
  "messages": [
    {"role": "system", "content": "..."},
    {"role": "user", "content": "..."},
    {"role": "assistant", "content": null, "tool_calls": [...]},
    {"role": "tool", "tool_call_id": "...", "content": "..."}
  ],
  "stream": true,
  "tools": [...]
}
```

#### `codex-api/src/sse/chat.rs`（717 行）

Chat Completions SSE 流式解析器。核心逻辑：

- 解析 `data: {...}` 和 `data: [DONE]` 事件
- 处理 `choices[0].delta.content` → `ResponseEvent::OutputTextDelta`
- 处理 `choices[0].delta.tool_calls` → `ResponseEvent::OutputItemDone(FunctionCall)`
- 处理 `choices[0].delta.reasoning` → `ResponseEvent::ReasoningContentDelta`
- 工具调用参数跨 delta 拼接（`ToolCallState` 状态机）
- 多 choice 并行处理

关键方法：`spawn_chat_stream()` → 启动异步任务处理 SSE 流

#### `codex-api/src/endpoint/chat.rs`（~160 行）

Chat 客户端封装。核心逻辑：

- 端点路径：`"chat/completions"`（line 67-72）
- 调用 `ChatRequestBuilder` 构建请求
- 调用 `spawn_chat_stream` 处理 SSE 响应

#### `core/src/tools/spec.rs` 中的转换函数（~40 行）

`create_tools_json_for_chat_completions_api()`：将 Responses API 工具格式转换为 Chat Completions 格式。

转换逻辑：
```rust
// Responses API 格式
{"type": "function", "name": "demo", "description": "...", "parameters": {...}}

// Chat Completions 格式
{"type": "function", "name": "demo", "function": {"name": "demo", "description": "...", "parameters": {...}}}
```

#### `core/src/client.rs` 中的路由逻辑（~60 行）

`stream_chat_completions()` 方法：

- 构建认证（`auth_manager` → `api_auth`）
- 创建 `ApiChatClient`
- 调用 `client.stream_prompt()`
- 处理 401 未授权重试

路由分支：
```rust
match wire_api {
    WireApi::Responses => { /* ... */ }
    WireApi::Chat => {
        let api_stream = self.stream_chat_completions(prompt).await?;
        // 根据 show_raw_agent_reasoning 决定是否聚合
    }
}
```

### 3.3 测试文件

- `core/tests/chat_completions_payload.rs`（338 行）：请求载荷测试
- `core/tests/chat_completions_sse.rs`（466 行）：SSE 解析测试

### 3.4 内部数据模型（未变）

以下类型在删除 Chat 支持后仍然存在且结构基本未变：

- `ResponseItem`（`codex-protocol/src/models.rs`）：内部统一的消息表示
- `ResponseEvent`（`codex-api/src/common.rs`）：流式事件枚举
- `FunctionCall`、`FunctionCallOutput` 等工具调用类型
- `ContentItem`（`InputText`、`OutputText`、`InputImage`）

---

## 四、难点分析

### 4.1 核心难点：重新引入 Chat Completions 适配层

| 维度 | Responses API（当前） | Chat Completions（DeepSeek） |
|------|---------------------|---------------------------|
| 端点 | `/v1/responses` | `/v1/chat/completions` |
| 请求体 | `input` 数组（ResponseItem） | `messages` 数组 |
| 流式事件 | `response.output_text.delta` | `choices[0].delta.content` |
| 工具调用 | output 中的 `function_call` | `choices[0].message.tool_calls` |
| 推理能力 | `reasoning` 字段 | 不同格式或不支持 |

### 4.2 功能降级矩阵

| 功能 | Responses API | Chat Completions | 处理策略 |
|------|--------------|------------------|----------|
| 基础对话 | ✅ | ✅ | 直接支持 |
| 工具调用 | ✅ | ✅ | 格式转换 |
| 流式输出 | ✅ | ✅ | SSE 解析适配 |
| 推理过程 | ✅ | ⚠️ 部分 | 需要检测支持 |
| 图片生成 | ✅ | ❌ | 降级禁用 |
| Web 搜索 | ✅ | ❌ | 降级禁用 |
| WebSocket 传输 | ✅ | ❌ | 回退 SSE |
| 命名空间工具 | ✅ | ❌ | 降级禁用 |
| 远程压缩 | ✅ | ❌ | 降级禁用 |

### 4.3 Onboarding UI 扩展

需要新增第四种登录选项："配置第三方 API"

收集信息：
- API Base URL
- API Key
- 模型名称
- 验证连接可用性

### 4.4 Provider 结构体变化

历史版本中 `Provider` 有 `wire: WireApi` 字段，当前版本已移除。需要评估是否恢复。

---

## 五、实施建议

### 5.1 推荐方案：基于历史代码恢复

**优势**：
- 核心逻辑（1200+ 行）可直接复用
- 内部数据模型未变，适配工作量小
- 测试用例可作为验证基准

**工作量评估**：

| 任务 | 行数 | 说明 |
|------|------|------|
| 恢复 `requests/chat.rs` | ~494 | 请求构建器，可能需要微调签名 |
| 恢复 `sse/chat.rs` | ~717 | SSE 解析器，可能需要适配新事件类型 |
| 恢复 `endpoint/chat.rs` | ~160 | 客户端封装 |
| 恢复工具格式转换 | ~40 | `create_tools_json_for_chat_completions_api` |
| 恢复 `client.rs` 路由 | ~60 | `stream_chat_completions()` 方法 |
| 适配 Provider 结构体 | ~50 | `wire` 字段恢复 |
| 适配认证流 | ~100 | 新的 `AuthManager` 接口 |
| Onboarding UI | ~200 | 新增第三方 API 选项 |
| 配置系统 | ~50 | `WireApi::Chat` 反序列化恢复 |
| 测试恢复/适配 | ~500 | 测试用例 |
| **总计** | **~2370** | 对比从零实现减少 ~40% |

### 5.2 分阶段实施

#### 阶段 1：WireApi 恢复（~100 行）

1. 在 `model-provider-info/src/lib.rs` 恢复 `WireApi::Chat` 变体
2. 修改 `Deserialize` 实现，移除报错逻辑
3. 在 `Provider` 结构体中恢复 `wire` 字段
4. 更新 `to_api_provider()` 映射

#### 阶段 2：请求构建器恢复（~500 行）

1. 恢复 `codex-api/src/requests/chat.rs`
2. 适配新的 `ResponseItem` 类型变化
3. 恢复 `create_tools_json_for_chat_completions_api()`

#### 阶段 3：SSE 解析器恢复（~750 行）

1. 恢复 `codex-api/src/sse/chat.rs`
2. 适配新的 `ResponseEvent` 变体
3. 恢复 `codex-api/src/endpoint/chat.rs`

#### 阶段 4：客户端路由恢复（~200 行）

1. 恢复 `client.rs` 中的 `WireApi::Chat` 分支
2. 恢复 `stream_chat_completions()` 方法
3. 适配新的认证流

#### 阶段 5：功能降级处理（~150 行）

1. 检测 Chat 模式下的不支持功能
2. 在 `ProviderCapabilities` 中正确设置标志
3. 在 UI 中隐藏不支持的功能

#### 阶段 6：Onboarding UI 扩展（~250 行）

1. 新增 `SignInOption::ThirdPartyApi` 变体
2. 添加配置输入界面（URL、Key、Model）
3. 连接验证逻辑
4. 配置持久化

#### 阶段 7：测试与验证（~500 行）

1. 恢复历史测试用例
2. 适配到新的测试框架
3. 添加 DeepSeek 集成测试

### 5.3 快速验证路径

如果 DeepSeek 未来添加 Responses API 支持（类似 Ollama 0.13.4+），则只需：

```toml
# ~/.codex/config.toml
model_provider = "deepseek"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
env_key = "DEEPSEEK_API_KEY"
wire_api = "responses"  # 如果 DeepSeek 支持
```

无需任何代码修改。

---

## 六、关键文件索引

### 配置系统

| 文件 | 作用 |
|------|------|
| `codex-rs/config/src/config_toml.rs` | TOML 配置解析 |
| `codex-rs/config/src/types.rs` | 配置类型定义 |
| `codex-rs/core/src/config/mod.rs` | 配置加载与合并 |

### 模型提供商

| 文件 | 作用 |
|------|------|
| `codex-rs/model-provider-info/src/lib.rs` | 提供商定义与 WireApi |
| `codex-rs/model-provider/src/provider.rs` | 运行时提供商抽象 |
| `codex-rs/model-provider/src/auth.rs` | 提供商认证解析 |

### API 客户端

| 文件 | 作用 |
|------|------|
| `codex-rs/codex-api/src/common.rs` | API 类型定义（ResponsesApiRequest 等） |
| `codex-rs/codex-api/src/endpoint/responses.rs` | Responses 客户端 |
| `codex-rs/codex-api/src/sse/responses.rs` | Responses SSE 解析器 |
| `codex-rs/core/src/client.rs` | 核心客户端路由 |

### 认证系统

| 文件 | 作用 |
|------|------|
| `codex-rs/login/src/auth/manager.rs` | 认证管理器 |
| `codex-rs/login/src/auth/storage.rs` | 认证存储 |
| `codex-rs/cli/src/login.rs` | CLI 登录命令 |

### TUI 界面

| 文件 | 作用 |
|------|------|
| `codex-rs/tui/src/onboarding/auth.rs` | 登录界面 |
| `codex-rs/tui/src/onboarding/onboarding_screen.rs` | Onboarding 流程 |
| `codex-rs/tui/src/lib.rs` | TUI 主入口 |

### 历史代码（可通过 git 恢复）

```bash
# 查看被删除的文件
git show d2394a249^:codex-rs/codex-api/src/requests/chat.rs
git show d2394a249^:codex-rs/codex-api/src/sse/chat.rs
git show d2394a249^:codex-rs/codex-api/src/endpoint/chat.rs

# 查看完整 diff
git show d2394a249 --stat

# 查看特定文件的变更
git show d2394a249 -- codex-rs/core/src/client.rs
git show d2394a249 -- codex-rs/core/src/tools/spec.rs
```

---

## 七、config.toml 配置参考

### 第三方提供商配置示例

```toml
# 使用 DeepSeek
model_provider = "deepseek"

[model_providers.deepseek]
name = "DeepSeek V4"
base_url = "https://api.deepseek.com/v1"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat"  # 需要恢复支持
requires_openai_auth = false

# 使用 OpenRouter
model_provider = "openrouter"

[model_providers.openrouter]
name = "OpenRouter"
base_url = "https://openrouter.ai/api/v1"
env_key = "OPENROUTER_API_KEY"
wire_api = "chat"
requires_openai_auth = false

# 使用本地 Ollama（已有支持）
model_provider = "ollama"

[model_providers.ollama]
name = "Ollama Local"
# base_url 默认 http://localhost:11434/v1
# wire_api 默认 responses（需要 Ollama 0.13.4+）
```

### 环境变量配置

```bash
# DeepSeek
export DEEPSEEK_API_KEY="sk-..."

# OpenRouter
export OPENROUTER_API_KEY="sk-..."

# 自定义提供商
export MY_PROVIDER_API_KEY="sk-..."
```

---

## 八、注意事项

1. **不要修改 `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` 或 `CODEX_SANDBOX_ENV_VAR` 相关代码**
2. **运行 `just fmt` 格式化代码**
3. **运行 `just test -p <project>` 测试特定项目**
4. **运行 `just fix -p <project>` 修复 linter 问题**
5. **修改 `ConfigToml` 后运行 `just write-config-schema` 更新 schema**
6. **修改 Rust 依赖后运行 `just bazel-lock-update` 更新 lockfile**

---

## 九、参考链接

- OpenAI Chat Completions API: https://platform.openai.com/docs/api-reference/chat
- OpenAI Responses API: https://platform.openai.com/docs/api-reference/responses
- DeepSeek API 文档: https://platform.deepseek.com/api-docs
- Chat 支持移除讨论: https://github.com/openai/codex/discussions/7782
- Ollama Responses API 支持: https://github.com/ollama/ollama/releases/tag/v0.13.4

---

## 十、实际开发记录

> 开发日期：2026-07-22  
> 执行方式：规划者（当前对话）直接编码 + 审核者（task 子代理）code review  
> 最终新增：24 files, +1572 lines, -8 lines

### 10.1 执行策略调整

原计划使用"开发者 task 子代理"执行代码，但遇到网络超时问题，改为**规划者直接编码 + 审核者 review**的模式。审核者成功发现 2 个 P0 阻断问题、1 个 proto 同步缺口、1 个测试逻辑过期问题。

### 10.2 各阶段实际执行

#### 阶段 1：WireApi::Chat 枚举恢复（commit: `a17210152`）

**修改文件**：5 files

| 文件 | 变更 |
|------|------|
| `model-provider-info/src/lib.rs` | 恢复 `WireApi::Chat`；更新 `Display`/`Deserialize`/`to_api_provider`；默认值保持 `Responses` |
| `model-provider-info/src/model_provider_info_tests.rs` | 重写测试：验证 `"chat"` 正确反序列化 |
| `codex-api/src/provider.rs` | 新增 `WireApi` 枚举 + `Provider.wire` 字段 |
| `codex-api/src/lib.rs` | 导出 `WireApi` |
| `core/src/client.rs` | `match wire_api` 添加 `WireApi::Chat` 占位分支 |

**遇到的问题**：
- 默认值从 `Responses` 改为 `Chat` 导致 4 个测试失败 → 保持 `Responses` 为默认值
- `CHAT_WIRE_API_REMOVED_ERROR` 删除时误删了 `AMAZON_BEDROCK_*` 和 `OLLAMA_*` 常量 → 手动恢复

**审核发现**：
- P0：测试引用已删除常量 → 已修复
- P0：match 不穷尽 → 已修复
- P1：proto 定义缺少 Chat → 暂不处理（remote config 不需要）
- P2：默认值兼容 → 保持 `Responses` 为默认

#### 阶段 2a：Chat 请求构建器（commit: `85fd7d7d6`）

**修改文件**：12 files（新增 `requests/chat.rs` 514 行 + 各处添加 `wire` 字段）

**遇到的问题**（类型变化适配）：
1. `build_conversation_headers` → `build_session_headers`（API 已重构）
2. `FunctionCallOutputPayload.content`/`.content_items` → `.body`（改为 `FunctionCallOutputBody` 枚举）
3. `ContentItem::InputImage` 新增 `detail` 字段
4. `ResponseItem` 新增 `AdditionalTools`、`AgentMessage`、`ToolSearchCall` 等变体
5. `ResponseItem::Message` 移除 `end_turn`，新增 `internal_chat_message_metadata_passthrough`
6. `ResponseItem::FunctionCall` 新增 `namespace`、`internal_chat_message_metadata_passthrough`
7. `ResponseItem::FunctionCallOutput` 新增 `id`、`internal_chat_message_metadata_passthrough`
8. `ResponseItem::GhostSnapshot` 被移除
9. `ContentItem::InputAudio` 新增
10. 新增 `Provider.wire` 字段导致所有 Provider 构造处编译失败 → 批量添加

**适配方法**：对不可穷尽 match 使用 `_ => {}` 通配符，对新增字段使用 `..` 模式

#### 阶段 2b：工具格式转换（commit: `6ed3a0a97`）

**修改文件**：2 files（`tools/src/tool_spec.rs` + `lib.rs`）

**变化**：原位于 `core/src/tools/spec.rs`，现已重构到 `codex-tools` crate

#### 阶段 3：Chat SSE 解析器（commit: `8ba4a70dd`）

**修改文件**：2 files（新增 `sse/chat.rs` 722 行 + `sse/mod.rs`）

**遇到的问题**（类型变化适配）：
1. `ResponseStream` 新增 `upstream_request_id` 字段
2. `ResponseEvent::Completed` 新增 `end_turn` 字段
3. `ResponseEvent::OutputItemAdded` 仍然是 tuple variant（不是 struct），无需修改
4. `ResponseItem::Reasoning.id` 类型从 `String` 变为 `Option<ResponseItemId>`
5. `ResponseItem::Reasoning` 新增 `internal_chat_message_metadata_passthrough`

**适配方法**：手动精确修复，使用 Python 脚本辅助批量替换后用 `edit_file` 微调

#### 阶段 3b+4：ChatClient + 路由集成（commit: `4038b9e46`）

**修改文件**：5 files（新增 `endpoint/chat.rs` 104 行 + 路由集成）

**变化**：历史 `endpoint/chat.rs` 依赖 `StreamingClient`，该类已被移除。改为参考 `ResponsesClient` 模式，使用 `EndpointSession`。

**遇到的问题**：
1. `conversation_id` 字段不存在 → 使用 `responses_metadata.session_id`
2. `session_source` 类型不匹配 → 包裹 `Some()`
3. `ChatRequestBuilder::build()` 返回 `ApiError` → 使用 `codex_api::map_api_error` 转换

#### 阶段 5：功能降级（commit: `4aed22d74`）

**修改文件**：2 files

| 修改 | 说明 |
|------|------|
| `supports_remote_compaction` 增加 `WireApi::Responses` 检查 | Chat 提供商不支持远程压缩 |
| `capabilities()` 对 Chat 禁用 `namespace_tools/image_generation/web_search` | Chat API 不支持这些特性 |

#### 阶段 6：Onboarding UI

**结论：无需修改**。第三方 API 通过 `config.toml` + `env_key` 环境变量配置是标准方式。设置 `requires_openai_auth = false` 会自动跳过登录屏幕。

#### 阶段 7：测试

**结论：已覆盖**。`wire_api = "chat"` 的反序列化测试已更新；现有测试套件覆盖了核心配置和路由路径。

### 10.3 统计数据

| 指标 | 数值 |
|------|------|
| 总 commits | 6 |
| 修改文件 | 24 |
| 新增行数 | 1572 |
| 删除行数 | 8 |
| 新增文件 | 4（`requests/chat.rs`、`sse/chat.rs`、`endpoint/chat.rs`、`scripts/fix_chat.py`） |

### 10.4 关键经验

1. **Git 历史是宝贵的资源**：被删除的 Chat 代码结构完整，直接复用节省了大量时间
2. **类型变化是主要挑战**：5 个月间 `ResponseItem`、`ResponseEvent`、`FunctionCallOutputPayload` 等核心类型引入了多个新字段和变体，需要逐个适配
3. **审核者模式有效**：子代理 review 发现了 4 个关键问题（包括 2 个阻断问题），避免了提交后才发现
4. **Python 脚本辅助批量修改**：对于重复性修改（如添加 `wire` 字段、适配类型变化），使用 Python 脚本比手动逐个编辑高效
5. **`#[default]` 的位置很重要**：默认值从 `Responses` 改为 `Chat` 导致 4 个测试失败，保持 `Responses` 为默认值确保向后兼容

### 10.5 用户配置指南

```bash
# 1. 设置 API Key
export DEEPSEEK_API_KEY="sk-..."

# 2. 配置 ~/.codex/config.toml
cat >> ~/.codex/config.toml << 'EOF'
model_provider = "deepseek"

[model_providers.deepseek]
name = "DeepSeek V4"
base_url = "https://api.deepseek.com/v1"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat"
requires_openai_auth = false
EOF

# 3. 正常启动 codex
codex
```
