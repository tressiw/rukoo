//! `voice/scripts/speaker-assets.ps1` and `speaker-assets.mjs` fetch the model the tests label
//! with. It has to be the file the app verifies, so its pin must match `ModelSpec::nemotron()` in
//! voice-core. voice-core isn't a dependency of this crate, so both are compared as text.

use std::fs;
use std::path::PathBuf;

fn voice_file(path: &str) -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    fs::read_to_string(&file).unwrap_or_else(|error| panic!("read {}: {error}", file.display()))
}

/// The first string literal after `marker` in `text`.
fn literal_after<'a>(text: &'a str, marker: &str) -> &'a str {
    let rest = &text[text.find(marker).unwrap_or_else(|| panic!("no {marker}")) + marker.len()..];
    let start = rest.find('"').expect("an opening quote") + 1;
    let len = rest[start..].find('"').expect("a closing quote");
    &rest[start..start + len]
}

#[test]
fn the_assets_script_fetches_the_model_voice_core_pins() {
    let registry = voice_file("crates/voice-core/src/local/registry.rs");
    let start = registry
        .find("pub fn nemotron()")
        .expect("ModelSpec::nemotron() in registry.rs");
    let spec = &registry[start..];
    let spec = &spec[..spec[1..].find("pub fn ").map_or(spec.len(), |end| end + 1)];

    let revision = literal_after(spec, "const REVISION: &str =");
    let name = literal_after(spec, "file(");
    assert_eq!(name, "Nemotron-3-Diarization.q8_0.gguf");
    let after_name = &spec[spec.find(name).unwrap() + name.len()..];
    let size: String = after_name
        .trim_start_matches(|c: char| c == '"' || c == ',' || c.is_whitespace())
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '_')
        .filter(char::is_ascii_digit)
        .collect();
    let sha256 = literal_after(after_name, ",");
    assert_eq!(sha256.len(), 64, "{sha256}");

    let url =
        format!("https://huggingface.co/nvidia/Nemotron-3-Diarization/resolve/{revision}/{name}");
    for script in ["scripts/speaker-assets.ps1", "scripts/speaker-assets.mjs"] {
        let text = voice_file(script);
        assert!(text.contains(&url), "{script} should fetch {url}");
        assert!(
            text.contains(sha256),
            "{script} should check SHA-256 {sha256}"
        );
        assert!(
            text.contains(&format!("NemotronSize = {size}"))
                || text.contains(&format!("NEMOTRON_SIZE = {size}")),
            "{script} should expect {size} bytes"
        );
    }
}

#[test]
fn the_tests_read_the_ami_files_the_manifest_pins() {
    let manifest: serde_json::Value =
        serde_json::from_str(&voice_file("fixtures/ami/manifest.json")).unwrap();
    let names: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"ami_en2002d_2132.wav"), "{names:?}");
    // The excerpt is 60 s of 16 kHz mono 16-bit audio after the canonical 44-byte header, which
    // tests/runtime.rs relies on.
    let wav = &manifest["files"][names
        .iter()
        .position(|name| *name == "ami_en2002d_2132.wav")
        .unwrap()];
    assert_eq!(wav["size"], serde_json::json!(44 + 60 * 16_000 * 2));
}
