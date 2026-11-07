use std::fs;
use std::io::Write;

use codex_protocol::ThreadId;
use tempfile::TempDir;

use super::*;

#[tokio::test]
async fn backs_up_and_removes_later_duplicate_ordinals() {
    let temp_dir = TempDir::new().expect("temp dir");
    let rollout_path = temp_dir.path().join("rollout.jsonl");
    let original = concat!(
        "{\"ordinal\":0,\"value\":\"meta\"}\n",
        "{\"ordinal\":1,\"value\":\"first\"}\n",
        "{\"ordinal\":1,\"value\":\"duplicate\"}\n",
        "{\"ordinal\":2,\"value\":\"next\"}\n",
    );
    fs::write(&rollout_path, original).expect("write rollout");

    let backup_path = repair_duplicate_ordinals(ThreadId::default(), &rollout_path)
        .await
        .expect("repair rollout")
        .expect("backup path");

    assert_eq!(
        fs::read_to_string(&rollout_path).expect("read repaired rollout"),
        concat!(
            "{\"ordinal\":0,\"value\":\"meta\"}\n",
            "{\"ordinal\":1,\"value\":\"first\"}\n",
            "{\"ordinal\":2,\"value\":\"next\"}\n",
        )
    );
    assert_eq!(
        fs::read_to_string(backup_path).expect("read backup"),
        original
    );
}

#[tokio::test]
async fn does_not_create_backup_for_unique_ordinals() {
    let temp_dir = TempDir::new().expect("temp dir");
    let rollout_path = temp_dir.path().join("rollout.jsonl");
    let mut file = fs::File::create(&rollout_path).expect("create rollout");
    writeln!(file, "{{\"ordinal\":0}}\n{{\"ordinal\":1}}").expect("write rollout");

    assert!(
        repair_duplicate_ordinals(ThreadId::default(), &rollout_path)
            .await
            .expect("scan rollout")
            .is_none()
    );
    assert_eq!(
        fs::read_dir(temp_dir.path())
            .expect("list directory")
            .count(),
        1
    );
}

#[tokio::test]
async fn preserves_unparsed_records_and_unterminated_last_line() {
    let temp_dir = TempDir::new().expect("temp dir");
    let rollout_path = temp_dir.path().join("rollout.jsonl");
    let original =
        b"{\"ordinal\":3}\r\ninvalid json\n{\"ordinal\":3}\n{\"ordinal\":-1}\n{\"ordinal\":4}";
    fs::write(&rollout_path, original).expect("write rollout");

    let backup = repair_duplicate_ordinals(ThreadId::default(), &rollout_path)
        .await
        .expect("repair rollout")
        .expect("backup path");

    assert_eq!(
        fs::read(&rollout_path).expect("read repaired rollout"),
        b"{\"ordinal\":3}\r\ninvalid json\n{\"ordinal\":-1}\n{\"ordinal\":4}"
    );
    assert_eq!(fs::read(backup).expect("read backup"), original);
}
