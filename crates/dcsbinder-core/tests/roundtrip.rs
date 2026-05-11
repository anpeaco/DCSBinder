//! Byte-equal round-trip test for the `.diff.lua` parser + serializer.
//!
//! For every `.diff.lua` and `modifiers.lua` in `tests/fixtures/`, this test:
//! 1. Reads the file as raw bytes.
//! 2. Parses it into a `DiffFile`.
//! 3. Serializes the `DiffFile` back to a string.
//! 4. Asserts the serialized bytes equal the source bytes exactly.
//!
//! This is the load-bearing correctness guarantee for the parser. Failing this
//! test means a future remap operation that re-serializes (M5+ merge feature)
//! could produce a file DCS rejects. Per ADR-003, the M1–M3 remap path avoids
//! re-serialization entirely as a mitigation.

use std::path::{Path, PathBuf};

use dcsbinder_core::parser;

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn collect_fixture_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        panic!("read_dir({}) failed: {e}", dir.display());
    });
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "lua") {
            out.push(path);
        }
    }
}

#[test]
fn byte_equal_roundtrip_every_fixture() {
    let fixtures = collect_fixture_files(&fixtures_root());
    assert!(
        !fixtures.is_empty(),
        "no fixture files found under {}",
        fixtures_root().display()
    );

    let mut failures: Vec<String> = Vec::new();

    for path in &fixtures {
        let bytes = std::fs::read(path).expect("read fixture");
        let source = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(e) => {
                failures.push(format!("{}: not valid UTF-8: {e}", path.display()));
                continue;
            }
        };

        let parsed = match parser::parse(&source) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{}: parse failed: {e}", path.display()));
                continue;
            }
        };

        let re_serialized = parser::write(&parsed);

        if re_serialized.as_bytes() != bytes {
            let diff = first_difference(re_serialized.as_bytes(), &bytes);
            failures.push(format!(
                "{}: round-trip differs at byte {} (original={:?}, written={:?})",
                path.display(),
                diff.offset,
                diff.original_window,
                diff.written_window,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} fixtures failed round-trip:\n{}",
        failures.len(),
        fixtures.len(),
        failures.join("\n")
    );
}

struct Difference {
    offset: usize,
    original_window: String,
    written_window: String,
}

fn first_difference(written: &[u8], original: &[u8]) -> Difference {
    let mut offset = 0;
    let max = written.len().min(original.len());
    while offset < max && written[offset] == original[offset] {
        offset += 1;
    }
    let window = 40usize;
    let start = offset.saturating_sub(window / 2);
    let end_o = (offset + window / 2).min(original.len());
    let end_w = (offset + window / 2).min(written.len());
    Difference {
        offset,
        original_window: String::from_utf8_lossy(&original[start..end_o]).into_owned(),
        written_window: String::from_utf8_lossy(&written[start..end_w]).into_owned(),
    }
}
