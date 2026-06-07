use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn production_source_contains_no_box_leak_outside_atom_table_and_tests() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    scan_dir(&src, &mut violations);

    assert!(
        violations.is_empty(),
        "Box::leak is forbidden in beamr production source except atom/table.rs. \
         Test-helper files and #[cfg(test)] blocks are allowed. Violations:\n{}",
        violations.join("\n")
    );
}

fn scan_dir(dir: &Path, violations: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        violations.push(format!("{}: unable to read directory", dir.display()));
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, violations);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            scan_file(&path, violations);
        }
    }
}

fn scan_file(path: &Path, violations: &mut Vec<String>) {
    let path_text = path.to_string_lossy();
    if path_text.ends_with("src/atom/table.rs") || is_test_helper_file(path) {
        return;
    }

    let Ok(source) = fs::read_to_string(path) else {
        violations.push(format!("{}: unable to read file", path.display()));
        return;
    };

    let mut cfg_test_parent_depth: Option<usize> = None;
    let mut pending_cfg_test = false;
    let mut brace_depth = 0usize;

    for (line_index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
        }

        let allowed_by_cfg_test = cfg_test_parent_depth.is_some() || pending_cfg_test;
        if line.contains("Box::leak") && !allowed_by_cfg_test {
            violations.push(format!("{}:{}:{}", path.display(), line_index + 1, trimmed));
        }

        let opens = line.chars().filter(|ch| *ch == '{').count();
        let closes = line.chars().filter(|ch| *ch == '}').count();
        let parent_depth = brace_depth;
        brace_depth = brace_depth.saturating_add(opens).saturating_sub(closes);

        if pending_cfg_test && opens > closes {
            cfg_test_parent_depth = Some(parent_depth);
        }
        pending_cfg_test = pending_cfg_test && opens == 0;

        if cfg_test_parent_depth.is_some_and(|depth| brace_depth <= depth) {
            cfg_test_parent_depth = None;
        }
    }
}

fn is_test_helper_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
}
