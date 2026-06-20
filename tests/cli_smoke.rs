use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn init_creates_layout() {
    let dir = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("noema").unwrap();
    cmd.env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized"));
    assert!(dir.path().join("tenants/personal/hippocampus").exists());
}

#[test]
fn remember_routes_to_review_queue() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["remember", "Prefer Rust for Noema.", "--tag", "rust"])
        .assert()
        .success()
        .stdout(predicate::str::contains("queued"));
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .assert()
        .success()
        .stdout(predicate::str::contains("Prefer Rust for Noema."));
}

#[test]
fn personal_mode_rejects_confidential_memory() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args([
            "remember",
            "Confidential launch plan.",
            "--sensitivity",
            "confidential",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "confidential and restricted sensitivity require enterprise mode",
        ));
}

#[test]
fn edit_and_merge_are_event_sourced_review_actions() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("init")
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["remember", "Prefer Rust for Noema."])
        .assert()
        .success();

    let first = Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .output()
        .unwrap();
    assert!(first.status.success());
    let first_stdout = String::from_utf8(first.stdout).unwrap();
    let first_candidate_id = first_stdout.split_whitespace().next().unwrap().to_string();
    let target_memory_id = first_candidate_id.replacen("cand_", "mem_", 1);

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["accept", &first_candidate_id])
        .assert()
        .success();

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args(["remember", "Prefer Rust for Noema again."])
        .assert()
        .success();

    let second = Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .output()
        .unwrap();
    assert!(second.status.success());
    let second_stdout = String::from_utf8(second.stdout).unwrap();
    let candidate_id = second_stdout.split_whitespace().next().unwrap().to_string();

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args([
            "edit",
            &candidate_id,
            "--body",
            "Prefer Rust for Noema memory.",
            "--reason",
            "clarify",
        ])
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .assert()
        .success()
        .stdout(predicate::str::contains("Prefer Rust for Noema memory."));

    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .args([
            "merge",
            &candidate_id,
            &target_memory_id,
            "--reason",
            "duplicate",
        ])
        .assert()
        .success();
    Command::cargo_bin("noema")
        .unwrap()
        .env("NOEMA_ROOT", dir.path())
        .arg("review")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}
