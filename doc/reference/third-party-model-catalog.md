# Third-Party Model Catalog: Adding Custom Models Without Recompilation

> **Context**: This document was created during a session where we added Xiaomi Mimo V2.5 models alongside existing DeepSeek V4 models to Codex CLI's TUI model picker (`/model` command), without modifying Rust source code or recompiling.

---

## Architecture Overview

Codex loads available models from two sources, controlled by `model_catalog_json` in `config.toml`:

```
┌─ model_catalog_json not set ──────────────────────────────┐
│  OpenAiModelsManager (default)                             │
│    ┌─ Remote: calls provider's GET /models endpoint        │
│    └─ Fallback: uses compiled-in models.json from          │
│       codex-rs/models-manager/models.json                  │
│    ⚠ Remote fetch is SKIPPED for env_key-only providers    │
│      (no uses_codex_backend / has_command_auth)            │
└───────────────────────────────────────────────────────────┘

┌─ model_catalog_json = "<path>" ───────────────────────────┐
│  StaticModelsManager                                       │
│    Loads models from the JSON file directly                │
│    Completely replaces the compiled-in models.json         │
│    No remote /models endpoint calls                        │
└───────────────────────────────────────────────────────────┘
```

**Key insight**: `model_catalog_json` is **exclusive** — it fully replaces the built-in catalog. If you want both built-in models AND custom ones, you must include all models in your custom JSON file.

---

## Key Configuration Files

### 1. `~/.codex/config.toml` — Provider & Model Catalog Config

```toml
# ============================================================
# Model Provider Definitions
# ============================================================

[model_providers.mimo]
name = "Mimo"
base_url = "https://api.xiaomimimo.com/v1"
env_key = "MIMO_API_KEY"
wire_api = "chat"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat"

# ============================================================
# Model Selection
# ============================================================

# Which provider to use from the model_providers map
# model_provider = "mimo"

# Optional model override (defaults to first model in catalog)
# model = "mimo-v2.5-pro"

# Optional: path to a JSON model catalog (replaces compiled-in models)
# model_catalog_json = "/home/kokoro/.codex/third_catalog.json"
```

### 2. `model_catalog_json` — Model Catalog JSON File

```json
{
  "models": [
    {
      "slug": "deepseek-v4-flash",
      "display_name": "DeepSeek V4 Flash",
      "description": "Fast and affordable AI-powered coding from DeepSeek.",
      "visibility": "list",
      "supported_in_api": true,
      "priority": 50,
      "shell_type": "shell_command",
      "truncation_policy": { "mode": "tokens", "limit": 10000 },
      "supports_parallel_tool_calls": true,
      "support_verbosity": false,
      "base_instructions": "You are Codex, an AI coding assistant powered by DeepSeek V4 Flash.",
      "apply_patch_tool_type": "freeform",
      "web_search_tool_type": "text",
      "context_window": 65536,
      "max_context_window": 65536,
      "use_responses_lite": false,
      "input_modalities": ["text"],
      "supported_reasoning_levels": [
        { "effort": "low", "description": "Fast responses with lighter reasoning" },
        { "effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks" },
        { "effort": "high", "description": "Greater reasoning depth for complex problems" }
      ],
      "default_reasoning_level": "medium",
      "experimental_supported_tools": []
    }
  ]
}
```

> ⚠️ **`InputModality` enum** (defined in `codex-rs/protocol/src/openai_models.rs`):
> Only accepts `"text"`, `"image"`, `"audio"`. `"video"` will cause a JSON parse error:
> ```
> Error loading configuration: failed to parse model_catalog_json path `...` as JSON:
> unknown variant `video`, expected one of `text`, `image`, `audio` at line ...
> ```

---

## The Model/Provider Decoupling Problem

This is the **most critical design detail** to understand:

```
model_provider  → determines the API endpoint (base_url)
model           → determines the "model" string sent in the API body
```

These two are **independently configurable**. The TUI `/model` command changes ONLY the `model` field. It does NOT change `model_provider`.

This means if you:
- Set `model_provider = "deepseek"` (base_url = `https://api.deepseek.com`)
- Then switch `/model` to `mimo-v2.5`

The actual API request sent will be:
```
POST https://api.deepseek.com/chat/completions
{
  "model": "mimo-v2.5",
  ...
}
```

DeepSeek's API will reject this because it doesn't know about `mimo-v2.5`.

**The error message might look like:**
```
{"error":{"message":"The supported API model names are deepseek-v4-pro or deepseek-v4-flash,
but you passed mimo-v2.5.","type":"invalid_request_error",...}}
```

---

## Solution: Local Routing Proxy

Since Codex doesn't support per-model provider switching at runtime (without recompilation), the recommended solution is a lightweight local proxy that routes requests based on the `model` field.

### Architecture

```
Codex → http://localhost:9090/v1/chat/completions (single provider)
                │
                ├─ model starts with "deepseek-" → api.deepseek.com
                └─ model starts with "mimo-"     → api.xiaomimimo.com
```

### Config Setup

```toml
[model_providers.myrouter]
name = "MyRouter"
base_url = "http://localhost:9090/v1"
wire_api = "chat"

model_provider = "myrouter"
model_catalog_json = "/home/kokoro/.codex/third_catalog.json"
```

### Proxy Script (`router.py`)

See the companion script at `./codex-references/router-proxy.py` for a working implementation.

Key features:
- Routes based on model name prefix (`deepseek-*`, `mimo-*`)
- Respects per-provider API keys via environment variables
- Handles streaming (Content-Type passthrough)
- Implements a `GET /models` endpoint for model discovery

---

## Model Parameter Reference

### DeepSeek V4

| Parameter | deepseek-v4-flash | deepseek-v4-pro |
|---|---|---|
| **Base URL** | `https://api.deepseek.com` | same |
| **API protocol** | OpenAI Chat / Anthropic | same |
| **Context window** | 64K (65,536) | 64K (65,536) |
| **Thinking mode** | ✅ `thinking: {type: "enabled"}` + `reasoning_effort` | same |
| **Model slug** | `deepseek-v4-flash` | `deepseek-v4-pro` |
| **Old names** | `deepseek-chat` → non-thinking mode | — |
| | `deepseek-reasoner` → thinking mode | — |
| **Deprecation** | `deepseek-chat`/`deepseek-reasoner` deprecated 2026-07-24 | — |

> Docs: https://api-docs.deepseek.com/zh-cn/quick_start/pricing

### Xiaomi Mimo V2.5

| Parameter | mimo-v2.5 | mimo-v2.5-pro |
|---|---|---|
| **Base URL** | `https://api.xiaomimimo.com/v1` | same |
| **API protocol** | OpenAI Chat / Anthropic | same |
| **Context window** | **1M** (1,048,576) | **1M** (1,048,576) |
| **Max output** | 128K tokens | 128K tokens |
| **Multimodal** | ✅ text/image/video/audio | ✅ text/image/video/audio |
| **Thinking mode** | ✅ `extra_body: {thinking: {type: "disabled\|enabled"}}` | same |
| **Model slug** | `mimo-v2.5` | `mimo-v2.5-pro` |
| **Description** | Native full-modal perception, multi-modal Agent scenarios | Trillion-parameter flagship, agent task excellence |

> Docs: https://mimo.mi.com/models/zh-CN/mimo-v2.5
>       https://mimo.mi.com/models/zh-CN/mimo-v2.5-pro

---

## Key Source Files Referenced

| File | Purpose |
|---|---|
| `codex-rs/model-provider-info/src/lib.rs` | `ModelProviderInfo` struct, `built_in_model_providers()`, `merge_configured_model_providers()`, `WireApi` enum |
| `codex-rs/model-provider/src/provider.rs` | `ModelProvider` trait, `create_model_provider()`, `ConfiguredModelProvider` |
| `codex-rs/models-manager/src/manager.rs` | `ModelsManager` trait, `OpenAiModelsManager`, `StaticModelsManager`, `RefreshStrategy` |
| `codex-rs/models-manager/src/lib.rs` | `bundled_models_response()` — loads compiled `models.json` |
| `codex-rs/models-manager/models.json` | Compiled-in model catalog (contains deepseek models as of this writing) |
| `codex-rs/protocol/src/openai_models.rs` | `ModelInfo`, `ModelPreset`, `InputModality` enum |
| `codex-rs/core/src/config/mod.rs` | Config loading, `model_providers` parsing, `model_catalog_json` loading |
| `codex-rs/tui/src/app_server_session.rs` | `bootstrap()` — fetches `model/list` RPC on TUI startup |
| `codex-rs/app-server/src/request_processors/catalog_processor.rs` | `list_models()` — handles the `model/list` RPC |
| `codex-rs/app-server/src/models.rs` | `supported_models()` — filters and transforms models for API response |

---

## Important Design Rules

1. **`model_providers` extend, built-in providers don't override**: User-defined providers in `config.toml`'s `model_providers` map are added as new entries. Built-in IDs (like `openai`, `amazon-bedrock`) can't be replaced — only `amazon-bedrock` allows limited overrides (`base_url`, `auth`, `http_headers`, `aws.*`).

2. **`wire_api` is either `"responses"` or `"chat"`**: For third-party providers that speak OpenAI-compatible Chat API, use `wire_api = "chat"`.

3. **`model_catalog_json` is exclusive, not additive**: It replaces `models.json`. To see all models, include everything in one file.

4. **Provider determines base_url, model determines the body parameter**: These are independent. `/model` only changes the latter.
