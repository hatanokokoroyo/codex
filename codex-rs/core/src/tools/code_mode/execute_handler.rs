use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde_json::Value as JsonValue;

use super::ExecContext;
use super::PUBLIC_TOOL_NAME;
use super::handle_runtime_response;
use super::is_exec_tool_name;

pub struct CodeModeExecuteHandler {
    spec: ToolSpec,
    nested_tool_specs: Vec<ToolSpec>,
}

impl CodeModeExecuteHandler {
    pub(crate) fn new(spec: ToolSpec, nested_tool_specs: Vec<ToolSpec>) -> Self {
        Self {
            spec,
            nested_tool_specs,
        }
    }

    async fn execute(
        &self,
        session: std::sync::Arc<crate::session::session::Session>,
        turn: std::sync::Arc<crate::session::turn_context::TurnContext>,
        call_id: String,
        code: String,
    ) -> Result<FunctionToolOutput, FunctionCallError> {
        let args =
            codex_code_mode::parse_exec_source(&code).map_err(FunctionCallError::RespondToModel)?;
        let exec = ExecContext { session, turn };
        let enabled_tools =
            codex_tools::collect_code_mode_tool_definitions(&self.nested_tool_specs);
        let started_at = std::time::Instant::now();
        let started_cell = exec
            .session
            .services
            .code_mode_service
            .execute(codex_code_mode::ExecuteRequest {
                tool_call_id: call_id.clone(),
                enabled_tools,
                source: args.code.clone(),
                yield_time_ms: args.yield_time_ms,
                max_output_tokens: args.max_output_tokens,
            })
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        let cell_id = started_cell.cell_id.clone();
        let runtime_cell_id = cell_id.to_string();
        let code_cell_trace = exec
            .session
            .services
            .rollout_thread_trace
            .start_code_cell_trace(
                exec.turn.sub_id.as_str(),
                runtime_cell_id.as_str(),
                call_id.as_str(),
                args.code.as_str(),
            );
        exec.session
            .services
            .code_mode_service
            .mark_cell_ready_for_dispatch(&cell_id);
        let response = started_cell
            .initial_response()
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        // Record the raw runtime boundary. The model-visible custom-tool output
        // is produced by `handle_runtime_response` and later linked through
        // `CodeCell.output_item_ids` in the reduced trace.
        code_cell_trace.record_initial_response(&response);
        // Yielded cells keep running, so terminal lifecycle is only emitted
        // here when the first response also ended the runtime.
        if !matches!(response, codex_code_mode::RuntimeResponse::Yielded { .. }) {
            code_cell_trace.record_ended(&response);
            exec.session
                .services
                .code_mode_service
                .finish_cell_dispatch(&cell_id);
        }
        exec.session.services.elicitations.wait_until_clear().await;
        handle_runtime_response(&exec, response, args.max_output_tokens, started_at)
            .await
            .map_err(FunctionCallError::RespondToModel)
    }
}

impl ToolExecutor<ToolInvocation> for CodeModeExecuteHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(PUBLIC_TOOL_NAME)
    }

    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(self.handle_call(invocation))
    }
}

/// Extracts the JavaScript source code from the payload. The exec tool is
/// registered as either a Freeform (`ToolPayload::Custom`) or Function
/// (`ToolPayload::Function` with a `code` string field) tool depending on the
/// wire API.
fn extract_code_from_payload(
    payload: &ToolPayload,
    tool_name: &ToolName,
) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Custom { input } if is_exec_tool_name(tool_name) => Ok(input.clone()),
        ToolPayload::Function { arguments } if is_exec_tool_name(tool_name) => {
            let parsed: JsonValue = serde_json::from_str(arguments).map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to parse exec arguments: {err}"))
            })?;
            match parsed.get("code").and_then(JsonValue::as_str) {
                Some(code) => Ok(code.to_string()),
                None => Err(FunctionCallError::RespondToModel(
                    "exec function call requires a \"code\" string parameter".to_string(),
                )),
            }
        }
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{PUBLIC_TOOL_NAME} expects raw JavaScript source text"
        ))),
    }
}

impl CodeModeExecuteHandler {
    async fn handle_call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            tool_name,
            payload,
            ..
        } = invocation;

        let code = extract_code_from_payload(&payload, &tool_name)?;
        self.execute(session, turn, call_id, code)
            .await
            .map(boxed_tool_output)
    }
}

impl CoreToolRuntime for CodeModeExecuteHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(
            payload,
            ToolPayload::Custom { .. } | ToolPayload::Function { .. }
        )
    }
}
