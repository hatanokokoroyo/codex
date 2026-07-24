use super::*;
use codex_model_provider_info::WireApi;
use codex_tools::ToolName;
use pretty_assertions::assert_eq;

#[test]
fn create_code_mode_tool_responses_api_returns_freeform() {
    let enabled_tools = vec![codex_code_mode::ToolDefinition {
        name: "update_plan".to_string(),
        tool_name: ToolName::plain("update_plan"),
        description: "Update the plan".to_string(),
        kind: codex_code_mode::CodeModeToolKind::Function,
        input_schema: None,
        output_schema: None,
    }];

    let spec = create_code_mode_tool(
        &enabled_tools,
        &[],
        &BTreeMap::new(),
        codex_code_mode::DEFAULT_EXEC_YIELD_TIME_MS,
        /*code_mode_only*/ true,
        WireApi::Responses,
    );
    assert!(matches!(spec, ToolSpec::Freeform(_)));
    assert_eq!(spec.name(), "exec");
}

#[test]
fn create_code_mode_tool_chat_api_returns_function() {
    let enabled_tools = vec![codex_code_mode::ToolDefinition {
        name: "update_plan".to_string(),
        tool_name: ToolName::plain("update_plan"),
        description: "Update the plan".to_string(),
        kind: codex_code_mode::CodeModeToolKind::Function,
        input_schema: None,
        output_schema: None,
    }];

    let spec = create_code_mode_tool(
        &enabled_tools,
        &[],
        &BTreeMap::new(),
        codex_code_mode::DEFAULT_EXEC_YIELD_TIME_MS,
        /*code_mode_only*/ true,
        WireApi::Chat,
    );
    assert!(matches!(spec, ToolSpec::Function(_)));
    assert_eq!(spec.name(), "exec");

    // Verify the function has a "code" parameter with correct schema.
    if let ToolSpec::Function(ref tool) = spec {
        let params = &tool.parameters;
        assert!(params.properties.is_some());
        let props = params.properties.as_ref().unwrap();
        assert!(props.contains_key("code"));
        assert_eq!(params.required, Some(vec!["code".to_string()]));
    } else {
        panic!("expected Function spec");
    }
}
