use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[test]
fn invalid_mappings_report_the_offending_declaration() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/invalid");
    let output_dir = fixture.join("target");
    std::fs::create_dir_all(&output_dir).unwrap();
    let scoop = if cfg!(windows) { "sco" } else { "scoop" };
    let output = Command::new(scoop)
        .arg("check")
        .arg("--release")
        .arg(&fixture)
        .arg("--lockfile")
        .arg(output_dir.join("Cone.lock"))
        .args(["--message-format", "json", "--generator"])
        .arg(concat!(
            "scoop.db=",
            env!("CARGO_BIN_EXE_scoop-db-generator")
        ))
        .output()
        .expect("run scoop check on the invalid models");
    assert!(!output.status.success());
    let diagnostics = String::from_utf8(output.stderr).unwrap();
    let mut actual = Vec::new();
    for line in diagnostics.lines() {
        let diagnostic: Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("{diagnostics}"));
        // The frontend also reports each interface method that could not be generated.
        if diagnostic["code"] == "E1008" {
            continue;
        }
        assert_eq!(diagnostic["code"], "GENERATOR_DIAGNOSTIC", "{line}");
        assert_eq!(diagnostic["origin"]["path"], "src/models.scoop");
        actual.push((
            diagnostic["display"]["line"].as_u64().unwrap(),
            diagnostic["display"]["column"].as_u64().unwrap(),
            diagnostic["message"].as_str().unwrap().to_owned(),
        ));
    }
    actual.sort();
    let expected = [
        (5, 67, "duplicate database column `id`"),
        (10, 24, "DbColumn emitNull requires an Option field"),
        (
            15,
            55,
            "omitted parameter `hidden` requires a default or Option",
        ),
        (
            20,
            28,
            "DbColumn ignore cannot be combined with other settings",
        ),
        (
            27,
            28,
            "no database column mapping for `Unmapped`; provide a ColumnCodec or RowReader companion",
        ),
        (
            32,
            30,
            "multiple matching ColumnCodec constructor dependencies",
        ),
        (
            41,
            5,
            "row generation requires the primary constructor to be the only visible constructor",
        ),
    ];
    assert_eq!(
        actual,
        expected.map(|(line, col, message)| (line, col, message.to_owned()))
    );
}
