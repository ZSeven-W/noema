use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn mcp_lists_noema_tools() {
    let mut cmd = Command::cargo_bin("noema-mcp").unwrap();
    cmd.write_stdin(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
        .assert()
        .success()
        .stdout(predicate::str::contains("noema_recall"))
        .stdout(predicate::str::contains("noema_review_decide"));
}

#[test]
fn mcp_calls_status_tool() {
    let mut cmd = Command::cargo_bin("noema-mcp").unwrap();
    cmd.write_stdin(
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"noema_status","arguments":{}}}"#,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("\"tenant\":\"personal\""));
}
