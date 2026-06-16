#[test]
fn end_to_end_text_output() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_genechart"))
        .arg("tests/fixtures/sample.ged")
        .arg("--pref")
        .arg("output.type = \"text\"")
        .output()
        .expect("failed to run genechart");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.is_empty(), "expected non-empty output");
    assert!(
        stdout.contains("John"),
        "expected John in output:\n{stdout}"
    );
}

#[test]
fn end_to_end_unknown_output_type_falls_back_to_text() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_genechart"))
        .arg("tests/fixtures/sample.ged")
        .arg("--pref")
        .arg("output.type = \"unknown\"")
        .output()
        .expect("failed to run genechart");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.is_empty());
}

#[test]
fn end_to_end_unknown_realistic_tree_style_errors() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_genechart"))
        .arg("tests/fixtures/sample.ged")
        .arg("--type")
        .arg("boxed_couples")
        .arg("--svg")
        .arg("--pref")
        .arg("output.style.realistic_tree.enabled = true")
        .arg("--pref")
        .arg("output.style.realistic_tree.style = \"bogus\"")
        .output()
        .expect("failed to run genechart");
    assert!(
        !output.status.success(),
        "expected non-zero exit for unknown realistic_tree style"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown output.style.realistic_tree.style") && stderr.contains("bogus"),
        "expected a clear error message, got:\n{stderr}"
    );
}
