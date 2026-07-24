use codex_features::Feature;
use codex_features::Features;
use codex_protocol::openai_models::ModelInfo;

pub(super) fn unsupported_code_mode_warning(
    model_info: &ModelInfo,
    features: &Features,
) -> Option<String> {
    let code_mode_enabled =
        features.enabled(Feature::CodeMode) || features.enabled(Feature::CodeModeOnly);
    if !code_mode_enabled
        || model_info.tool_mode.is_some()
        || model_info.used_fallback_model_metadata
    {
        return None;
    }

    // When code mode is enabled via config features, effective_tool_mode()
    // resolves to CodeMode even when the model's own metadata does not set
    // tool_mode. The runtime will use Code Mode through the config fallback,
    // so there is no mismatch to warn about.
    None
}

#[cfg(test)]
#[path = "code_mode_warning_tests.rs"]
mod tests;
