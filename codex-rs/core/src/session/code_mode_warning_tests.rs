use super::unsupported_code_mode_warning;
use codex_features::Feature;
use codex_features::Features;
use codex_models_manager::model_info::model_info_from_slug;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ToolMode;
use pretty_assertions::assert_eq;

const MODEL_SLUG: &str = "test-model";

fn known_model_info() -> ModelInfo {
    ModelInfo {
        used_fallback_model_metadata: false,
        ..model_info_from_slug(MODEL_SLUG)
    }
}

/// When code mode is enabled via config features, effective_tool_mode()
/// resolves to CodeMode even when the model's own metadata does not set
/// tool_mode. The runtime will use Code Mode through the config fallback,
/// so there is no mismatch to warn about.
#[test]
fn no_warning_when_code_mode_enabled_without_model_selector() {
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);

    assert_eq!(
        unsupported_code_mode_warning(&known_model_info(), &features),
        None
    );
}

#[test]
fn no_warning_when_code_mode_only_enabled_without_model_selector() {
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeModeOnly);

    assert_eq!(
        unsupported_code_mode_warning(&known_model_info(), &features),
        None
    );
}

#[test]
fn does_not_warn_when_code_mode_is_disabled() {
    assert_eq!(
        unsupported_code_mode_warning(&known_model_info(), &Features::with_defaults()),
        None
    );
}

#[test]
fn does_not_warn_when_model_has_tool_mode_selector() {
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeModeOnly);

    for tool_mode in [ToolMode::Direct, ToolMode::CodeMode, ToolMode::CodeModeOnly] {
        let model_info = ModelInfo {
            tool_mode: Some(tool_mode),
            ..known_model_info()
        };
        assert_eq!(unsupported_code_mode_warning(&model_info, &features), None);
    }
}

#[test]
fn fallback_metadata_only_uses_existing_warning() {
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);

    assert_eq!(
        unsupported_code_mode_warning(&model_info_from_slug(MODEL_SLUG), &features),
        None
    );
}
