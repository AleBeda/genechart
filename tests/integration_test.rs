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

/// Configuring a plugin in a build without the `lua` feature must be a hard error.
#[cfg(not(feature = "lua"))]
#[test]
fn end_to_end_plugin_without_lua_feature_errors() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_genechart"))
        .arg("tests/fixtures/plugins_sample.ged")
        .arg("--text")
        .arg("--pref")
        .arg("plugins.parse.indi = \"tests/fixtures/plugins/nickname.lua\"")
        .output()
        .expect("failed to run genechart");
    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--features lua"),
        "expected a 'rebuild with --features lua' error, got:\n{stderr}"
    );
}

/// `plugins.parse.indi` rewrites the given name using the NICK tag.
#[cfg(feature = "lua")]
#[test]
fn end_to_end_lua_indi_plugin_rewrites_given_name() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_genechart"))
        .arg("tests/fixtures/plugins_sample.ged")
        .arg("--text")
        .arg("--pref")
        .arg("plugins.parse.indi = \"tests/fixtures/plugins/nickname.lua\"")
        .output()
        .expect("failed to run genechart");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Robert \"Bob\""),
        "expected the NICK-rewritten name, got:\n{stdout}"
    );
}

/// `plugins.parse.all` appends ", USA" to places ending in a US state code,
/// for both individuals (death) and families (marriage).
#[cfg(feature = "lua")]
#[test]
fn end_to_end_lua_all_plugin_appends_usa() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_genechart"))
        .arg("tests/fixtures/plugins_sample.ged")
        .arg("--text")
        .arg("--pref")
        .arg("plugins.parse.all = \"tests/fixtures/plugins/usa.lua\"")
        .output()
        .expect("failed to run genechart");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Chicago, IL, USA"),
        "expected the individual death place to gain ', USA', got:\n{stdout}"
    );
    assert!(
        stdout.contains("Reno, NV, USA"),
        "expected the family marriage place to gain ', USA', got:\n{stdout}"
    );
}
