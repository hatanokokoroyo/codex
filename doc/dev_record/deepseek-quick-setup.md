# DeepSeek 快速配置方案

> 设计日期：2026-07-22  
> 状态：已实施

---

## 一、目标

在 Codex CLI 首次启动时，用户只需输入 DeepSeek API Key，即可自动完成：
1. `config.toml` 的自动生成（含 DeepSeek V4 Flash/Pro 双模型配置）
2. API Key 的安全持久化
3. 后续启动直接跳过登录屏幕，无缝使用 DeepSeek

---

## 二、现状分析

### 2.1 当前启动流程

```
首次运行 codex（无 config.toml，无 auth.json）
  │
  ├─ 配置加载：默认 provider = openai, requires_openai_auth = true
  │
  ├─ 信任检查：should_show_trust_screen() → true
  │
  ├─ 登录检查：should_show_login_screen() → true（requires_openai_auth && !authenticated）
  │
  └─ 显示 Onboarding：Welcome → Auth → Trust
       │
       └─ AuthModeWidget 显示三个选项：
            • Sign in with ChatGPT
            • Sign in with Device Code
            • Provide your own API key
```

**关键文件**（实施后）：
| 文件 | 作用 |
|------|------|
| `tui/src/lib.rs:2009-2017` | `should_show_login_screen()` — 登录屏幕显示控制 |
| `tui/src/onboarding/auth.rs` | `SignInState`、`SignInOption`、DeepSeek 配置全流程 |
| `tui/src/onboarding/keys.rs` | `SELECT_FOURTH` 快捷键绑定 |
| `codex-api/src/requests/chat.rs` | `ChatRequestBuilder` — `developer`→`system` 角色映射 |
| `models-manager/models.json` | `deepseek-v4-flash`/`deepseek-v4-pro` 模型元数据注册 |

### 2.2 API Key 持久化路径

当前 OpenAI API Key 保存流程：
1. TUI 调用 `save_api_key()` → app-server → `login_with_api_key()`
2. 写入 `~/.codex/auth.json`：`{ "auth_mode": "api_key", "OPENAI_API_KEY": "sk-..." }`
3. 后续请求时从 `auth.json` 加载

**问题**：`auth.json` 只在 `requires_openai_auth = true` 时被读取。第三方 provider 设为 `false` 后，API Key 必须来自环境变量（`env_key`）或配置中的 `experimental_bearer_token`。

### 2.3 第三方 Provider 的 API Key 来源

当 `requires_openai_auth = false` 时，API Key 获取顺序（`model-provider/src/auth.rs`）：
1. `env_key` 环境变量
2. `experimental_bearer_token` 配置字段
3. `auth` 命令输出

---

## 三、实现方案

### 3.1 核心思路

在 Onboarding 的认证步骤中新增第四个选项："**Use DeepSeek**"，选择后进入简化的配置流程。

### 3.2 完整流程

```
首次运行 codex
  │
  └─ Onboarding → AuthModeWidget（4个选项）
       │
       ├─ "Sign in with ChatGPT"         → 浏览器 OAuth（已有）
       ├─ "Sign in with Device Code"     → 设备码登录（已有）
       ├─ "Provide your own API key"     → OpenAI API Key（已有）
       └─ "Use DeepSeek"                → DeepSeek 快速配置（新增）
            │
            ├─ 1. 输入 API Key（文本输入框，预置掩码显示）
            │     • 从 DEEPSEEK_API_KEY 环境变量预填充（如有）
            │     • 支持粘贴
            │
            ├─ 2. 选择默认模型（可选步骤）
            │     • deepseek-v4-flash（V4 Flash，快速廉价）[默认]
            │     • deepseek-v4-pro（V4 Pro，深度推理）
            │     • 两个都用（根据任务自动选择）
            │
            └─ 3. 确认 → 一键完成：
                  │
                  ├─ 写入 ~/.codex/config.toml：
                  │   model_provider = "deepseek"
                  │   model = "deepseek-v4-flash"
                  │
                  │   [model_providers.deepseek]
                  │   name = "DeepSeek"
                  │   base_url = "https://api.deepseek.com/v1"
                  │   wire_api = "chat"
                  │   requires_openai_auth = false
                  │   experimental_bearer_token = "sk-..."  ← 用户输入的 key
                  │
                  └─ 状态机跳转到 DeepSeekConfigured（= StepState::Complete）
```

### 3.3 SignInState 扩展

```rust
// tui/src/onboarding/auth.rs
pub(crate) enum SignInState {
    PickMode,
    ChatGptContinueInBrowser(ContinueInBrowserState),
    ChatGptDeviceCode(ContinueWithDeviceCodeState),
    ChatGptSuccessMessage,
    ChatGptSuccess,
    ApiKeyEntry(ApiKeyInputState),
    ApiKeyConfigured,
    // 新增 ↓
    DeepSeekEntry(DeepSeekInputState),      // API Key 输入中
    DeepSeekModelSelect(DeepSeekModelState), // 模型选择中
    DeepSeekConfigured,                     // 配置完成
}

#[derive(Clone, Default)]
pub(crate) struct DeepSeekInputState {
    value: String,
    prepopulated_from_env: bool,
}

#[derive(Clone)]
pub(crate) struct DeepSeekModelState {
    pub(crate) api_key: String,
    pub(crate) highlighted: DeepSeekModel,  // 当前高亮的模型选项
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeepSeekModel {
    Chat,       // deepseek-v4-flash
    Reasoner,   // deepseek-v4-pro
    Both,       // 两个都用
}
```

### 3.4 SignInOption 扩展

```rust
pub(crate) enum SignInOption {
    ChatGpt,
    DeviceCode,
    ApiKey,
    DeepSeek,  // 新增
}
```

选项显示逻辑扩展（`displayed_sign_in_options()`）：
```rust
fn displayed_sign_in_options(&self) -> Vec<SignInOption> {
    let mut options = vec![SignInOption::ChatGpt];
    if self.is_chatgpt_login_allowed() {
        options.push(SignInOption::DeviceCode);
    }
    if self.is_api_login_allowed() {
        options.push(SignInOption::ApiKey);
    }
    options.push(SignInOption::DeepSeek);  // 始终显示，不受 forced_login_method 限制
    options
}
```

### 3.5 配置写入

```rust
/// 写入 DeepSeek 的 config.toml 并持久化 API Key。
///
/// 检查 ~/.codex/config.toml 是否已存在：
/// - 不存在 → 创建新文件，写入完整 DeepSeek 配置
/// - 已存在 → 在现有文件中追加 [model_providers.deepseek] 段
async fn write_deepseek_config(
    codex_home: &Path,
    api_key: &str,
    model: DeepSeekModel,
) -> std::io::Result<()> {
    let config_path = codex_home.join("config.toml");
    let model_name = model.model_name();  // "deepseek-v4-flash" / "deepseek-v4-pro"

    let deepseek_config = format!(
        r#"
# DeepSeek provider — auto-generated by Codex setup
model_provider = "deepseek"
model = "{model}"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "chat"
requires_openai_auth = false
experimental_bearer_token = "{api_key}"
"#,
        model = model_name,
        api_key = api_key,
    );

    if config_path.exists() {
        let mut existing = tokio::fs::read_to_string(&config_path).await?;
        existing.push_str(&deepseek_config);
        tokio::fs::write(&config_path, existing).await?;
    } else {
        tokio::fs::write(&config_path, deepseek_config).await?;
    }
    Ok(())
}
```

### 3.6 模型配置（双模型支持）

当用户选择 "Both" 时，额外生成一个 profile：

```toml
# ~/.codex/config.toml
model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "chat"
requires_openai_auth = false
experimental_bearer_token = "sk-..."

[profile.deepseek-pro]
model = "deepseek-v4-pro"
model_provider = "deepseek"
```

用户可通过 `codex --profile deepseek-pro` 切换到推理模型。

### 3.7 键盘交互

DeepSeek 输入状态复用现有的 API Key 输入处理逻辑（`handle_api_key_entry_key_event`），但 target 改为 `DeepSeekEntry`。模型选择状态通过 MOVE_UP/MOVE_DOWN 切换 `highlighted` 模型，Enter 确认。

### 3.8 额外修复

实施过程中发现并修复了以下问题：

| 修复 | 文件 | 说明 |
|------|------|------|
| `developer`→`system` 角色映射 | `codex-api/src/requests/chat.rs` | Chat Completions API 不支持 `developer` 角色，映射为 `system` |
| DeepSeek V4 模型注册 | `models-manager/models.json` | 注册 `deepseek-v4-flash` 和 `deepseek-v4-pro` 元数据，消除 "model metadata not found" 告警 |

---

## 四、Security Note

`experimental_bearer_token` 以明文存储在 `config.toml` 中。建议用户改用 `env_key` + 环境变量：

```toml
# 推荐方式（替换 experimental_bearer_token）
env_key = "DEEPSEEK_API_KEY"
# experimental_bearer_token = "sk-..."  # 删除此行
```

```bash
# ~/.zshrc 或 ~/.bashrc
export DEEPSEEK_API_KEY="sk-..."
```

---

## 五、实际变更统计

| 文件 | 变更内容 |
|------|----------|
| `tui/src/onboarding/auth.rs` | ~260 行新增：DeepSeek 状态机、配置写入、UI 渲染 |
| `tui/src/onboarding/keys.rs` | +1 行：`SELECT_FOURTH` |
| `codex-api/src/requests/chat.rs` | +1 行：`developer`→`system` 角色映射 |
| `models-manager/models.json` | +2 模型：`deepseek-v4-flash`、`deepseek-v4-pro` |

---

## 六、风险与应对

| 风险 | 应对 |
|------|------|
| `experimental_bearer_token` 明文存储 | 文档提示用户迁移到 `env_key`；后续可支持 keyring 存储 |
| 多次运行重复写入 config.toml | 当前为简单 append；后续可用 `toml_edit` 合并 |
| 与现有 `forced_login_method` 冲突 | DeepSeek 选项始终显示，不受 `forced_login_method` 限制 |
| Chat API 功能受限（无 WebSocket/图片/搜索） | `WireApi::Chat` 自动降级 capabilities |

---

## 七、使用示例

```
$ codex                    # 首次启动

┌──────────────────────────────────────────┐
│  Welcome to Codex                        │
│                                          │
│  Sign in with ChatGPT to use Codex       │
│  as part of your paid plan               │
│  or connect an API key                   │
│                                          │
│  > Sign in with ChatGPT                  │
│    Sign in with Device Code              │
│    Provide your own API key              │
│    Use DeepSeek                          │  ← 新增
│                                          │
│  [Enter] confirm  [1-4] select  [Esc] back │
└──────────────────────────────────────────┘

         ↓ 用户选择 "Use DeepSeek"

┌──────────────────────────────────────────┐
│  Configure DeepSeek                      │
│                                          │
│  Enter your DeepSeek API key:            │
│  sk-xxxxxxxxxxxxxxxxxxxxxxxxxxxx█        │
│                                          │
│  Get your API key at:                    │
│  https://platform.deepseek.com/api_keys │
│                                          │
│  [Enter] confirm  [Esc] back             │
└──────────────────────────────────────────┘

         ↓ 用户输入 key 并按 Enter

┌──────────────────────────────────────────┐
│  Select default model                    │
│                                          │
│  > deepseek-v4-flash (Flash, fast)      │
│      Fast and affordable, best for       │
│      everyday coding tasks               │
│                                          │
│    deepseek-v4-pro (Pro, deep reasoning)│
│      Advanced reasoning, better for      │
│      complex problem-solving             │
│                                          │
│    Use both (auto-select)                │
│                                          │
│  [Enter] confirm  [↑↓] navigate         │
└──────────────────────────────────────────┘

         ↓ 用户选择模型并按 Enter

┌──────────────────────────────────────────┐
│  ✓ DeepSeek configured successfully!     │
│                                          │
│  • API key saved                         │
│  • Config written to ~/.codex/config.toml│
│  • Default model: deepseek-v4-flash     │
│                                          │
│  [Enter] continue                        │
└──────────────────────────────────────────┘

         ↓ 进入正常 TUI 使用界面
```
