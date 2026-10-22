use std::process::{Command, Stdio};

fn glance() -> Command {
    Command::new(env!("CARGO_BIN_EXE_glance"))
}

#[test]
fn version_matches_cargo() {
    let output = glance().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("glance {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn snapshot_into_closed_pipe_does_not_panic() {
    for args in [&["snapshot"][..], &["snapshot", "--json"], &["about"]] {
        let mut child = glance()
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("panicked"), "glance {args:?}: {stderr}");
    }
}
