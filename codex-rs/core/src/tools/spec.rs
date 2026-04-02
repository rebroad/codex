use crate::shell::Shell;
use crate::shell::ShellType;
use crate::tools::code_mode::PUBLIC_TOOL_NAME;
use crate::tools::code_mode::WAIT_TOOL_NAME;
use crate::tools::handlers::agent_jobs::BatchJobHandler;
use crate::tools::handlers::multi_agents_common::DEFAULT_WAIT_TIMEOUT_MS;
use crate::tools::handlers::multi_agents_common::MAX_WAIT_TIMEOUT_MS;
use crate::tools::handlers::multi_agents_common::MIN_WAIT_TIMEOUT_MS;
use crate::tools::registry::ToolRegistryBuilder;
use codex_mcp::mcp::CODEX_APPS_MCP_SERVER_NAME;
use codex_mcp::mcp_connection_manager::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_tools::TOOL_SEARCH_TOOL_NAME;
use codex_tools::TOOL_SUGGEST_TOOL_NAME;
use codex_tools::DiscoverableTool;
use codex_tools::ToolHandlerKind;
use codex_tools::ToolRegistryPlanAppTool;
use codex_tools::ToolRegistryPlanParams;
use codex_tools::ToolSpec;
use codex_tools::ToolUserShellType;
use codex_tools::ToolsConfig;
use codex_tools::WaitAgentTimeoutOptions;
use codex_tools::augment_tool_spec_for_code_mode;
use codex_tools::build_tool_registry_plan;
use codex_tools::dynamic_tool_to_responses_api_tool;
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
pub(crate) use codex_tools::mcp_call_tool_result_output_schema;

pub(crate) fn tool_user_shell_type(user_shell: &Shell) -> ToolUserShellType {
    match user_shell.shell_type {
        ShellType::Zsh => ToolUserShellType::Zsh,
        ShellType::Bash => ToolUserShellType::Bash,
        ShellType::PowerShell => ToolUserShellType::PowerShell,
        ShellType::Sh => ToolUserShellType::Sh,
        ShellType::Cmd => ToolUserShellType::Cmd,
    }
}

fn push_tool_spec(
    builder: &mut ToolRegistryBuilder,
    spec: ToolSpec,
    supports_parallel_tool_calls: bool,
    code_mode_enabled: bool,
) {
    let spec = if code_mode_enabled {
        augment_tool_spec_for_code_mode(spec)
    } else {
        spec
    };
    if supports_parallel_tool_calls {
        builder.push_spec_with_parallel_support(spec, /*supports_parallel_tool_calls*/ true);
    } else {
        builder.push_spec(spec);
    }
}

fn canonical_builtin_tool_name(name: &str) -> Option<&str> {
    match name {
        "shell" | "local_shell" | "container.exec" | "shell_command" => Some("shell_command"),
        "exec_command" => Some("exec_command"),
        "write_stdin" => Some("write_stdin"),
        "update_plan" => Some("update_plan"),
        "request_user_input" => Some("request_user_input"),
        "request_permissions" => Some("request_permissions"),
        TOOL_SEARCH_TOOL_NAME => Some(TOOL_SEARCH_TOOL_NAME),
        TOOL_SUGGEST_TOOL_NAME => Some(TOOL_SUGGEST_TOOL_NAME),
        "apply_patch" => Some("apply_patch"),
        "web_search" => Some("web_search"),
        "image_generation" => Some("image_generation"),
        "view_image" => Some("view_image"),
        "spawn_agent" => Some("spawn_agent"),
        "send_input" => Some("send_input"),
        "send_message" => Some("send_message"),
        "assign_task" => Some("assign_task"),
        "resume_agent" => Some("resume_agent"),
        "wait_agent" => Some("wait_agent"),
        "close_agent" => Some("close_agent"),
        "list_agents" => Some("list_agents"),
        "spawn_agents_on_csv" => Some("spawn_agents_on_csv"),
        "report_agent_job_result" => Some("report_agent_job_result"),
        "list_mcp_resources" => Some("list_mcp_resources"),
        "list_mcp_resource_templates" => Some("list_mcp_resource_templates"),
        "read_mcp_resource" => Some("read_mcp_resource"),
        "js_repl" => Some("js_repl"),
        "js_repl_reset" => Some("js_repl_reset"),
        "list_dir" => Some("list_dir"),
        "test_sync_tool" => Some("test_sync_tool"),
        PUBLIC_TOOL_NAME => Some(PUBLIC_TOOL_NAME),
        WAIT_TOOL_NAME => Some(WAIT_TOOL_NAME),
        _ => None,
    }
}

fn builtin_tool_allowed(config: &ToolsConfig, canonical_name: &str) -> bool {
    if let Some(enabled_tools) = config.builtin_enabled_tools.as_ref()
        && !enabled_tools.contains(canonical_name)
    {
        return false;
    }

    !config.builtin_disabled_tools.contains(canonical_name)
}

fn apply_builtin_tool_policy(builder: &mut ToolRegistryBuilder, config: &ToolsConfig) {
    builder.retain_specs(|spec| {
        let Some(canonical_name) = canonical_builtin_tool_name(spec.name()) else {
            return true;
        };
        builtin_tool_allowed(config, canonical_name)
    });

    builder.retain_handlers(|name| {
        let Some(canonical_name) = canonical_builtin_tool_name(name) else {
            return true;
        };
        builtin_tool_allowed(config, canonical_name)
    });
}

/// Builds the tool registry builder while collecting tool specs for later serialization.
#[cfg(test)]
pub(crate) fn build_specs(
    config: &ToolsConfig,
    mcp_tools: Option<HashMap<String, rmcp::model::Tool>>,
    app_tools: Option<HashMap<String, ToolInfo>>,
    dynamic_tools: &[DynamicToolSpec],
) -> ToolRegistryBuilder {
    build_specs_with_discoverable_tools(
        config,
        mcp_tools,
        app_tools,
        /*discoverable_tools*/ None,
        dynamic_tools,
    )
}

pub(crate) fn build_specs_with_discoverable_tools(
    config: &ToolsConfig,
    mcp_tools: Option<HashMap<String, rmcp::model::Tool>>,
    app_tools: Option<HashMap<String, ToolInfo>>,
    discoverable_tools: Option<Vec<DiscoverableTool>>,
    dynamic_tools: &[DynamicToolSpec],
) -> ToolRegistryBuilder {
    use crate::tools::handlers::ApplyPatchHandler;
    use crate::tools::handlers::CodeModeExecuteHandler;
    use crate::tools::handlers::CodeModeWaitHandler;
    use crate::tools::handlers::DynamicToolHandler;
    use crate::tools::handlers::JsReplHandler;
    use crate::tools::handlers::JsReplResetHandler;
    use crate::tools::handlers::ListDirHandler;
    use crate::tools::handlers::McpHandler;
    use crate::tools::handlers::McpResourceHandler;
    use crate::tools::handlers::PlanHandler;
    use crate::tools::handlers::RequestPermissionsHandler;
    use crate::tools::handlers::RequestUserInputHandler;
    use crate::tools::handlers::ShellCommandHandler;
    use crate::tools::handlers::ShellHandler;
    use crate::tools::handlers::TestSyncHandler;
    use crate::tools::handlers::ToolSearchHandler;
    use crate::tools::handlers::ToolSuggestHandler;
    use crate::tools::handlers::UnifiedExecHandler;
    use crate::tools::handlers::ViewImageHandler;
    use crate::tools::handlers::multi_agents::CloseAgentHandler;
    use crate::tools::handlers::multi_agents::ResumeAgentHandler;
    use crate::tools::handlers::multi_agents::SendInputHandler;
    use crate::tools::handlers::multi_agents::SpawnAgentHandler;
    use crate::tools::handlers::multi_agents::WaitAgentHandler;
    use crate::tools::handlers::multi_agents_v2::AssignTaskHandler as AssignTaskHandlerV2;
    use crate::tools::handlers::multi_agents_v2::CloseAgentHandler as CloseAgentHandlerV2;
    use crate::tools::handlers::multi_agents_v2::ListAgentsHandler as ListAgentsHandlerV2;
    use crate::tools::handlers::multi_agents_v2::SendMessageHandler as SendMessageHandlerV2;
    use crate::tools::handlers::multi_agents_v2::SpawnAgentHandler as SpawnAgentHandlerV2;
    use crate::tools::handlers::multi_agents_v2::WaitAgentHandler as WaitAgentHandlerV2;

    let mut builder = ToolRegistryBuilder::new();
    let app_tool_sources = app_tools.as_ref().map(|app_tools| {
        app_tools
            .values()
            .map(|tool| ToolRegistryPlanAppTool {
                tool_name: tool.tool_name.as_str(),
                tool_namespace: tool.tool_namespace.as_str(),
                server_name: tool.server_name.as_str(),
                connector_name: tool.connector_name.as_deref(),
                connector_description: tool.connector_description.as_deref(),
            })
            .collect::<Vec<_>>()
    });
    let default_agent_type_description =
        crate::agent::role::spawn_tool_spec::build(&std::collections::BTreeMap::new());
    let plan = build_tool_registry_plan(
        config,
        ToolRegistryPlanParams {
            mcp_tools: mcp_tools.as_ref(),
            app_tools: app_tool_sources.as_deref(),
            discoverable_tools: discoverable_tools.as_deref(),
            dynamic_tools,
            default_agent_type_description: &default_agent_type_description,
            wait_agent_timeouts: WaitAgentTimeoutOptions {
                default_timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
                min_timeout_ms: MIN_WAIT_TIMEOUT_MS,
                max_timeout_ms: MAX_WAIT_TIMEOUT_MS,
            },
            codex_apps_mcp_server_name: CODEX_APPS_MCP_SERVER_NAME,
        },
    );
    let shell_handler = Arc::new(ShellHandler);
    let unified_exec_handler = Arc::new(UnifiedExecHandler);
    let plan_handler = Arc::new(PlanHandler);
    let apply_patch_handler = Arc::new(ApplyPatchHandler);
    let dynamic_tool_handler = Arc::new(DynamicToolHandler);
    let view_image_handler = Arc::new(ViewImageHandler);
    let mcp_handler = Arc::new(McpHandler);
    let mcp_resource_handler = Arc::new(McpResourceHandler);
    let shell_command_handler = Arc::new(ShellCommandHandler::from(config.shell_command_backend));
    let request_permissions_handler = Arc::new(RequestPermissionsHandler);
    let request_user_input_handler = Arc::new(RequestUserInputHandler {
        default_mode_request_user_input: config.default_mode_request_user_input,
    });
    let mut tool_search_handler = None;
    let tool_suggest_handler = Arc::new(ToolSuggestHandler);
    let code_mode_handler = Arc::new(CodeModeExecuteHandler);
    let code_mode_wait_handler = Arc::new(CodeModeWaitHandler);
    let js_repl_handler = Arc::new(JsReplHandler);
    let js_repl_reset_handler = Arc::new(JsReplResetHandler);

    for spec in plan.specs {
        if spec.supports_parallel_tool_calls {
            builder.push_spec_with_parallel_support(
                spec.spec, /*supports_parallel_tool_calls*/ true,
            );
        } else {
            builder.push_spec(spec.spec);
        }
    }

    for handler in plan.handlers {
        match handler.kind {
            ToolHandlerKind::AgentJobs => {
                builder.register_handler(handler.name, Arc::new(BatchJobHandler));
            }
            ToolHandlerKind::ApplyPatch => {
                builder.register_handler(handler.name, apply_patch_handler.clone());
            }
            ToolHandlerKind::AssignTaskV2 => {
                builder.register_handler(handler.name, Arc::new(AssignTaskHandlerV2));
            }
            ToolHandlerKind::CloseAgentV1 => {
                builder.register_handler(handler.name, Arc::new(CloseAgentHandler));
            }
            ToolHandlerKind::CloseAgentV2 => {
                builder.register_handler(handler.name, Arc::new(CloseAgentHandlerV2));
            }
            ToolHandlerKind::CodeModeExecute => {
                builder.register_handler(handler.name, code_mode_handler.clone());
            }
            ToolHandlerKind::CodeModeWait => {
                builder.register_handler(handler.name, code_mode_wait_handler.clone());
            }
            ToolHandlerKind::DynamicTool => {
                builder.register_handler(handler.name, dynamic_tool_handler.clone());
            }
            ToolHandlerKind::JsRepl => {
                builder.register_handler(handler.name, js_repl_handler.clone());
            }
            ToolHandlerKind::JsReplReset => {
                builder.register_handler(handler.name, js_repl_reset_handler.clone());
            }
            ToolHandlerKind::ListAgentsV2 => {
                builder.register_handler(handler.name, Arc::new(ListAgentsHandlerV2));
            }
            ToolHandlerKind::ListDir => {
                builder.register_handler(handler.name, Arc::new(ListDirHandler));
            }
            ToolHandlerKind::Mcp => {
                builder.register_handler(handler.name, mcp_handler.clone());
            }
            ToolHandlerKind::McpResource => {
                builder.register_handler(handler.name, mcp_resource_handler.clone());
            }
            ToolHandlerKind::Plan => {
                builder.register_handler(handler.name, plan_handler.clone());
            }
            ToolHandlerKind::RequestPermissions => {
                builder.register_handler(handler.name, request_permissions_handler.clone());
            }
            ToolHandlerKind::RequestUserInput => {
                builder.register_handler(handler.name, request_user_input_handler.clone());
            }
            ToolHandlerKind::ResumeAgentV1 => {
                builder.register_handler(handler.name, Arc::new(ResumeAgentHandler));
            }
            ToolHandlerKind::SendInputV1 => {
                builder.register_handler(handler.name, Arc::new(SendInputHandler));
            }
            ToolHandlerKind::SendMessageV2 => {
                builder.register_handler(handler.name, Arc::new(SendMessageHandlerV2));
            }
            ToolHandlerKind::Shell => {
                builder.register_handler(handler.name, shell_handler.clone());
            }
            ToolHandlerKind::ShellCommand => {
                builder.register_handler(handler.name, shell_command_handler.clone());
            }
            ToolHandlerKind::SpawnAgentV1 => {
                builder.register_handler(handler.name, Arc::new(SpawnAgentHandler));
            }
            ToolHandlerKind::SpawnAgentV2 => {
                builder.register_handler(handler.name, Arc::new(SpawnAgentHandlerV2));
            }
            ToolHandlerKind::TestSync => {
                builder.register_handler(handler.name, Arc::new(TestSyncHandler));
            }
            ToolHandlerKind::ToolSearch => {
                if tool_search_handler.is_none() {
                    tool_search_handler = app_tools
                        .as_ref()
                        .map(|app_tools| Arc::new(ToolSearchHandler::new(app_tools.clone())));
                }
                if let Some(tool_search_handler) = tool_search_handler.as_ref() {
                    builder.register_handler(handler.name, tool_search_handler.clone());
                }
            }
            ToolHandlerKind::ToolSuggest => {
                builder.register_handler(handler.name, tool_suggest_handler.clone());
            }
            ToolHandlerKind::UnifiedExec => {
                builder.register_handler(handler.name, unified_exec_handler.clone());
            }
            ToolHandlerKind::ViewImage => {
                builder.register_handler(handler.name, view_image_handler.clone());
            }
            ToolHandlerKind::WaitAgentV1 => {
                builder.register_handler(handler.name, Arc::new(WaitAgentHandler));
            }
            ToolHandlerKind::WaitAgentV2 => {
                builder.register_handler(handler.name, Arc::new(WaitAgentHandlerV2));
            }
        }
    }

    if !dynamic_tools.is_empty() {
        for tool in dynamic_tools {
            match dynamic_tool_to_responses_api_tool(tool) {
                Ok(converted_tool) => {
                    push_tool_spec(
                        &mut builder,
                        ToolSpec::Function(converted_tool),
                        /*supports_parallel_tool_calls*/ false,
                        config.code_mode_enabled,
                    );
                    builder.register_handler(tool.name.clone(), dynamic_tool_handler.clone());
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to convert dynamic tool {:?} to OpenAI tool: {e:?}",
                        tool.name
                    );
                }
            }
        }
    }

    apply_builtin_tool_policy(&mut builder, config);
    builder
}

#[cfg(test)]
#[path = "spec_tests.rs"]
mod tests;
