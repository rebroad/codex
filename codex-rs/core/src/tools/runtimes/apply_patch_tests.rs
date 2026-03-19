use super::*;
use codex_protocol::protocol::GranularApprovalConfig;
use pretty_assertions::assert_eq;
use std::collections::HashMap;

#[test]
fn wants_no_sandbox_approval_granular_respects_sandbox_flag() {
    let runtime = ApplyPatchRuntime::new();
    assert!(runtime.wants_no_sandbox_approval(AskForApproval::OnRequest));
    assert!(
        !runtime.wants_no_sandbox_approval(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: false,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        }))
    );
    assert!(
        runtime.wants_no_sandbox_approval(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        }))
    );
}

#[test]
fn guardian_review_request_includes_patch_context() {
    let path = std::env::temp_dir().join("guardian-apply-patch-test.txt");
    let action = ApplyPatchAction::new_add_for_test(&path, "hello".to_string());
    let expected_cwd = action.cwd.clone();
    let expected_patch = action.patch.clone();
    let request = ApplyPatchRequest {
        action,
        file_paths: vec![
            AbsolutePathBuf::from_absolute_path(&path).expect("temp path should be absolute"),
        ],
        changes: HashMap::from([(
            path,
            FileChange::Add {
                content: "hello".to_string(),
            },
        )]),
        exec_approval_requirement: ExecApprovalRequirement::NeedsApproval {
            reason: None,
            proposed_execpolicy_amendment: None,
        },
        sandbox_permissions: SandboxPermissions::UseDefault,
        additional_permissions: None,
        permissions_preapproved: false,
        timeout_ms: None,
        codex_exe: None,
    };

    let guardian_request = ApplyPatchRuntime::build_guardian_review_request(&request, "call-1");

    assert_eq!(
        guardian_request,
        GuardianApprovalRequest::ApplyPatch {
            id: "call-1".to_string(),
            cwd: expected_cwd,
            files: request.file_paths,
            change_count: 1usize,
            patch: expected_patch,
        }
    );
}

#[test]
fn resolve_deleted_exe_fallback_prefers_live_binary_when_present() {
    let root = std::env::temp_dir().join(format!(
        "apply-patch-runtime-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&root).expect("temp dir should be creatable");

    let live_exe = root.join("codex");
    std::fs::write(&live_exe, "#!/bin/sh\n").expect("test exe should be writable");
    let deleted_exe = root.join("codex (deleted)");

    let resolved = ApplyPatchRuntime::resolve_deleted_exe_fallback(deleted_exe);

    assert_eq!(resolved, live_exe);

    std::fs::remove_file(&live_exe).expect("temp file should be removable");
    std::fs::remove_dir(&root).expect("temp dir should be removable");
}

#[test]
fn resolve_deleted_exe_fallback_keeps_original_when_live_binary_missing() {
    let deleted_exe = std::env::temp_dir().join(format!(
        "apply-patch-runtime-missing-{}-{} (deleted)",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));

    let resolved = ApplyPatchRuntime::resolve_deleted_exe_fallback(deleted_exe.clone());

    assert_eq!(resolved, deleted_exe);
}
