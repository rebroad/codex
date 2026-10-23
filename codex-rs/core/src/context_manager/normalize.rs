use codex_context_fragments::set_annotated_content;
use codex_context_fragments::to_annotated_content;
use codex_history::CodexHarnessMetadata;
use codex_history::ResponseItemEnvelope;
use codex_protocol::ResponseItemId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::InputModality;
use std::collections::HashSet;
use uuid::Uuid;

use crate::context::ContextualUserFragment;
use crate::context::UnsupportedMedia;
use tracing::info;
use tracing::warn;

// Changing this value would change model-visible IDs and invalidate prompt caches.
const SYNTHETIC_OUTPUT_ID_NAMESPACE: Uuid = Uuid::from_u128(0x90d38d3e_6a5b_4d52_bfe2_2f1e634bfac4);

pub(crate) fn ensure_call_outputs_present(
    items: &mut Vec<ResponseItemEnvelope>,
) -> Vec<ResponseItemEnvelope> {
    restore_recovered_output_order(items);
    let mut function_output_ids = HashSet::new();
    let mut tool_search_output_ids = HashSet::new();
    let mut custom_tool_output_ids = HashSet::new();
    for envelope in items.iter() {
        match &envelope.item {
            ResponseItem::FunctionCallOutput {
                call_id: Some(call_id),
                ..
            } => {
                function_output_ids.insert(call_id.as_str());
            }
            ResponseItem::ToolSearchOutput {
                call_id: Some(call_id),
                ..
            } => {
                tool_search_output_ids.insert(call_id.as_str());
            }
            ResponseItem::CustomToolCallOutput { call_id, .. } => {
                custom_tool_output_ids.insert(call_id.as_str());
            }
            _ => {}
        }
    }

    // Collect synthetic outputs to insert immediately after their calls.
    // Store the insertion position (index of call) alongside the item so
    // we can insert in reverse order and avoid index shifting.
    let mut missing_outputs_to_insert: Vec<(usize, ResponseItemEnvelope)> = Vec::new();
    let mut recovered_outputs = Vec::new();

    for (idx, envelope) in items.iter().enumerate() {
        match &envelope.item {
            ResponseItem::FunctionCall { id, call_id, .. }
            | ResponseItem::LocalShellCall {
                id,
                call_id: Some(call_id),
                ..
            } if !function_output_ids.contains(call_id.as_str()) => {
                info!("Tool call output is missing for call id: {call_id}");
                let inherited = is_inherited(envelope);
                let mut output = ResponseItemEnvelope::new(ResponseItem::FunctionCallOutput {
                    id: synthetic_output_id("fco", id.as_deref()),
                    call_id: Some(call_id.clone()),
                    name: None,
                    namespace: None,
                    output: FunctionCallOutputPayload::from_text(missing_tool_output_message(
                        inherited,
                    )),
                    internal_chat_message_metadata_passthrough: None,
                });
                if !inherited {
                    record_recovered_tool_output(&mut output, call_id, &mut recovered_outputs);
                }
                missing_outputs_to_insert.push((idx, output));
            }
            ResponseItem::ToolSearchCall {
                id,
                call_id: Some(call_id),
                ..
            } if !tool_search_output_ids.contains(call_id.as_str()) => {
                info!("Tool search output is missing for call id: {call_id}");
                missing_outputs_to_insert.push((
                    idx,
                    ResponseItemEnvelope::new(ResponseItem::ToolSearchOutput {
                        id: synthetic_output_id("tso", id.as_deref()),
                        call_id: Some(call_id.clone()),
                        status: "completed".to_string(),
                        execution: "client".to_string(),
                        tools: Vec::new(),
                        internal_chat_message_metadata_passthrough: None,
                    }),
                ));
            }
            ResponseItem::CustomToolCall { id, call_id, .. }
                if !custom_tool_output_ids.contains(call_id.as_str()) =>
            {
                info!("Custom tool call output is missing for call id: {call_id}");
                // Preserved-prefix forks mark copied calls as inherited. A call still
                // running in the parent has no output in the child's snapshot.
                let inherited = is_inherited(envelope);
                let mut output = ResponseItemEnvelope::new(ResponseItem::CustomToolCallOutput {
                    id: synthetic_output_id("ctco", id.as_deref()),
                    call_id: call_id.clone(),
                    name: None,
                    output: FunctionCallOutputPayload::from_text(missing_tool_output_message(
                        inherited,
                    )),
                    internal_chat_message_metadata_passthrough: None,
                });
                if !inherited {
                    record_recovered_tool_output(&mut output, call_id, &mut recovered_outputs);
                }
                missing_outputs_to_insert.push((idx, output));
            }
            _ => {}
        }
    }
    drop((
        function_output_ids,
        tool_search_output_ids,
        custom_tool_output_ids,
    ));

    // Insert synthetic outputs in reverse index order to avoid re-indexing.
    for (idx, output_item) in missing_outputs_to_insert.into_iter().rev() {
        items.insert(idx + 1, output_item);
    }

    recovered_outputs
}

pub(crate) const MISSING_TOOL_OUTPUT_MESSAGE: &str = "Codex recovered an incomplete history record. The tool may have run, but its result was not saved. Inspect the current state before trying it again.";

fn missing_tool_output_message(inherited: bool) -> String {
    if inherited {
        "aborted".to_string()
    } else {
        MISSING_TOOL_OUTPUT_MESSAGE.to_string()
    }
}

fn is_inherited(envelope: &ResponseItemEnvelope) -> bool {
    envelope
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata.inherited_user_message)
}

fn mark_recovered_tool_output(output: &mut ResponseItemEnvelope) {
    output
        .metadata
        .get_or_insert_with(CodexHarnessMetadata::default)
        .recovered_tool_output = true;
}

fn record_recovered_tool_output(
    output: &mut ResponseItemEnvelope,
    call_id: &str,
    recovered_outputs: &mut Vec<ResponseItemEnvelope>,
) {
    mark_recovered_tool_output(output);
    recovered_outputs.push(output.clone());
    warn!(call_id = %call_id, "recovering missing tool call output");
}

/// Recovered output records are appended to the rollout so the original JSONL stays immutable.
/// Move them back beside their calls when reconstructing model history.
fn restore_recovered_output_order(items: &mut Vec<ResponseItemEnvelope>) {
    let mut recovered = Vec::new();
    let mut retained = Vec::with_capacity(items.len());
    for item in std::mem::take(items) {
        if item
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata.recovered_tool_output)
        {
            recovered.push(item);
        } else {
            retained.push(item);
        }
    }
    *items = retained;

    for output in recovered {
        let call_id = match &output.item {
            ResponseItem::FunctionCallOutput {
                call_id: Some(call_id),
                ..
            }
            | ResponseItem::CustomToolCallOutput { call_id, .. } => call_id,
            _ => {
                items.push(output);
                continue;
            }
        };
        let call_position = items.iter().position(|envelope| match &envelope.item {
            ResponseItem::FunctionCall { call_id: id, .. }
            | ResponseItem::LocalShellCall {
                call_id: Some(id), ..
            } => id == call_id,
            ResponseItem::CustomToolCall { call_id: id, .. } => id == call_id,
            _ => false,
        });
        if let Some(call_position) = call_position {
            items.insert(call_position + 1, output);
        } else {
            items.push(output);
        }
    }
}

/// Derives a stable ID for a synthesized output from its source call's item ID.
///
/// The namespace and name format must remain stable across retries and resumes
/// to preserve prompt-cache reuse. Returning `None` when the source call has no
/// ID preserves the legacy behavior for older history items.
fn synthetic_output_id(prefix: &str, item_id: Option<&str>) -> Option<ResponseItemId> {
    let source_id = item_id.filter(|id| !id.is_empty())?;
    let name = format!("{prefix}:{source_id}");
    Some(ResponseItemId::with_suffix(
        prefix,
        Uuid::new_v5(&SYNTHETIC_OUTPUT_ID_NAMESPACE, name.as_bytes()),
    ))
}

pub(crate) fn remove_orphan_outputs(items: &mut Vec<ResponseItemEnvelope>) -> usize {
    let mut function_call_ids = HashSet::new();
    let mut tool_search_call_ids = HashSet::new();
    let mut custom_tool_call_ids = HashSet::new();
    for envelope in items.iter() {
        match &envelope.item {
            ResponseItem::FunctionCall { call_id, .. }
            | ResponseItem::LocalShellCall {
                call_id: Some(call_id),
                ..
            } => {
                function_call_ids.insert(call_id.as_str());
            }
            ResponseItem::ToolSearchCall {
                call_id: Some(call_id),
                ..
            } => {
                tool_search_call_ids.insert(call_id.as_str());
            }
            ResponseItem::CustomToolCall { call_id, .. } => {
                custom_tool_call_ids.insert(call_id.as_str());
            }
            _ => {}
        }
    }

    let mut orphan_positions = Vec::new();
    for (position, envelope) in items.iter().enumerate() {
        match &envelope.item {
            ResponseItem::FunctionCallOutput {
                call_id: Some(call_id),
                ..
            } if !function_call_ids.contains(call_id.as_str()) => {
                warn!(call_id = %call_id, "Orphan function call output");
                orphan_positions.push(position);
            }
            ResponseItem::CustomToolCallOutput { call_id, .. }
                if !custom_tool_call_ids.contains(call_id.as_str()) =>
            {
                warn!(call_id = %call_id, "Orphan custom tool call output");
                orphan_positions.push(position);
            }
            ResponseItem::ToolSearchOutput {
                call_id: Some(call_id),
                execution,
                ..
            } if execution != "server" && !tool_search_call_ids.contains(call_id.as_str()) => {
                warn!(call_id = %call_id, "Orphan tool search output");
                orphan_positions.push(position);
            }
            _ => {}
        }
    }

    if !orphan_positions.is_empty() {
        let removed_count = orphan_positions.len();
        let mut orphan_positions = orphan_positions.into_iter().peekable();
        let mut position = 0;
        items.retain(|_| {
            let retain = orphan_positions.peek() != Some(&position);
            if !retain {
                orphan_positions.next();
            }
            position += 1;
            retain
        });
        removed_count
    } else {
        0
    }
}

pub(crate) fn remove_corresponding_for(items: &mut Vec<ResponseItemEnvelope>, item: &ResponseItem) {
    match item {
        ResponseItem::FunctionCall { call_id, .. } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::FunctionCallOutput {
                        call_id: Some(existing),
                        ..
                    } if existing == call_id
                )
            });
        }
        ResponseItem::FunctionCallOutput {
            call_id: Some(call_id),
            ..
        } => {
            if let Some(pos) = items.iter().position(|envelope| {
                matches!(&envelope.item, ResponseItem::FunctionCall { call_id: existing, .. } if existing == call_id)
            }) {
                items.remove(pos);
            } else if let Some(pos) = items.iter().position(|envelope| {
                matches!(&envelope.item, ResponseItem::LocalShellCall { call_id: Some(existing), .. } if existing == call_id)
            }) {
                items.remove(pos);
            }
        }
        ResponseItem::ToolSearchCall {
            call_id: Some(call_id),
            ..
        } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::ToolSearchOutput {
                        call_id: Some(existing),
                        ..
                    } if existing == call_id
                )
            });
        }
        ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            ..
        } => {
            remove_first_matching(
                items,
                |i| {
                    matches!(
                        i,
                        ResponseItem::ToolSearchCall {
                            call_id: Some(existing),
                            ..
                        } if existing == call_id
                    )
                },
            );
        }
        ResponseItem::CustomToolCall { call_id, .. } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::CustomToolCallOutput {
                        call_id: existing, ..
                    } if existing == call_id
                )
            });
        }
        ResponseItem::CustomToolCallOutput { call_id, .. } => {
            remove_first_matching(
                items,
                |i| matches!(i, ResponseItem::CustomToolCall { call_id: existing, .. } if existing == call_id),
            );
        }
        ResponseItem::LocalShellCall {
            call_id: Some(call_id),
            ..
        } => {
            remove_first_matching(items, |i| {
                matches!(
                    i,
                    ResponseItem::FunctionCallOutput {
                        call_id: Some(existing),
                        ..
                    } if existing == call_id
                )
            });
        }
        _ => {}
    }
}

fn remove_first_matching<F>(items: &mut Vec<ResponseItemEnvelope>, predicate: F)
where
    F: Fn(&ResponseItem) -> bool,
{
    if let Some(pos) = items.iter().position(|envelope| predicate(&envelope.item)) {
        items.remove(pos);
    }
}

/// Strip image content from messages and tool outputs when the model does not support images.
/// When `input_modalities` contains `InputModality::Image`, no stripping is performed.
pub(crate) fn strip_images_when_unsupported(
    input_modalities: &[InputModality],
    items: &mut [ResponseItemEnvelope],
) {
    let supports_images = input_modalities.contains(&InputModality::Image);
    if supports_images {
        return;
    }

    for envelope in items.iter_mut() {
        match &mut envelope.item {
            ResponseItem::Message { .. } => {
                let Some(mut content) = to_annotated_content(&mut envelope.item) else {
                    continue;
                };
                for content_item in &mut content {
                    if matches!(content_item.content(), ContentItem::InputImage { .. }) {
                        *content_item = UnsupportedMedia::IMAGE.render_fragment().into_parts().1;
                    }
                }
                let _ = set_annotated_content(&mut envelope.item, content);
            }
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                if let Some(content_items) = output.content_items_mut() {
                    let mut normalized_content_items = Vec::with_capacity(content_items.len());
                    for content_item in content_items.iter() {
                        match content_item {
                            FunctionCallOutputContentItem::InputImage { .. } => {
                                normalized_content_items.push(
                                    FunctionCallOutputContentItem::InputText {
                                        text: UnsupportedMedia::IMAGE.render(),
                                    },
                                );
                            }
                            _ => normalized_content_items.push(content_item.clone()),
                        }
                    }
                    *content_items = normalized_content_items;
                }
            }
            ResponseItem::ImageGenerationCall { result, .. } => {
                result.clear();
            }
            _ => {}
        }
    }
}

/// Strip audio content from messages and tool outputs when the model does not support audio.
/// When `input_modalities` contains `InputModality::Audio`, no stripping is performed.
pub(crate) fn strip_audio_when_unsupported(
    input_modalities: &[InputModality],
    items: &mut [ResponseItemEnvelope],
) {
    if input_modalities.contains(&InputModality::Audio) {
        return;
    }

    for envelope in items.iter_mut() {
        match &mut envelope.item {
            ResponseItem::Message { .. } => {
                let Some(mut content) = to_annotated_content(&mut envelope.item) else {
                    continue;
                };
                for content_item in &mut content {
                    if matches!(content_item.content(), ContentItem::InputAudio { .. }) {
                        *content_item = UnsupportedMedia::AUDIO.render_fragment().into_parts().1;
                    }
                }
                let _ = set_annotated_content(&mut envelope.item, content);
            }
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                if let Some(content_items) = output.content_items_mut() {
                    for content_item in content_items.iter_mut() {
                        if matches!(
                            content_item,
                            FunctionCallOutputContentItem::InputAudio { .. }
                        ) {
                            *content_item = FunctionCallOutputContentItem::InputText {
                                text: UnsupportedMedia::AUDIO.render(),
                            };
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
