use codex_code_mode::ToolDefinition as CodeModeToolDefinition;
use codex_model_provider_info::WireApi;
use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

/// Builds the `exec` tool definition. For Chat Completions API providers
/// (`wire_api == Chat`) the tool is emitted as a regular `function` so that
/// non-Responses-API endpoints can invoke it.  For Responses API providers it
/// remains a `Freeform` tool with a grammar-constrained format.
pub(crate) fn create_code_mode_tool(
    enabled_tools: &[CodeModeToolDefinition],
    deferred_tools: &[CodeModeToolDefinition],
    namespace_descriptions: &BTreeMap<String, codex_code_mode::ToolNamespaceDescription>,
    default_exec_yield_time_ms: u64,
    code_mode_only: bool,
    wire_api: WireApi,
) -> ToolSpec {
    let description = codex_code_mode::build_exec_tool_description(
        enabled_tools,
        deferred_tools,
        namespace_descriptions,
        default_exec_yield_time_ms,
        code_mode_only,
    );

    if wire_api == WireApi::Chat {
        // Chat Completions API does not support Freeform/custom tool types.
        // Wrap the exec tool as a standard function with a single `code` string
        // parameter so that the model can invoke it via normal function calling.
        return ToolSpec::Function(ResponsesApiTool {
            name: codex_code_mode::PUBLIC_TOOL_NAME.to_string(),
            description,
            strict: false,
            defer_loading: None,
            parameters: json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "JavaScript source code to execute in a V8 isolate. All nested tools are available on the global `tools` object (e.g. `await tools.exec_command(...)`). You may optionally start with a first-line pragma like `// @exec: {\"yield_time_ms\": 10000, \"max_output_tokens\": 1000}`."
                    }
                },
                "required": ["code"],
                "additionalProperties": false
            }),
            output_schema: None,
        });
    }

    const CODE_MODE_FREEFORM_GRAMMAR: &str = r#"
start: pragma_source | plain_source
pragma_source: PRAGMA_LINE NEWLINE SOURCE
plain_source: SOURCE

PRAGMA_LINE: /[ \t]*\/\/ @exec:[^\r\n]*/
NEWLINE: /\r?\n/
SOURCE: /[\s\S]+/
"#;

    ToolSpec::Freeform(FreeformTool {
        name: codex_code_mode::PUBLIC_TOOL_NAME.to_string(),
        description,
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition: CODE_MODE_FREEFORM_GRAMMAR.to_string(),
        },
    })
}

#[cfg(test)]
#[path = "execute_spec_tests.rs"]
mod tests;
