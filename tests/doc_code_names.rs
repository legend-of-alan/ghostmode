//! Every code name that `AGENTS.md`, `README.md` and `docs/*.md` write in backticks is real: a
//! `path/to/file.rs` names a file in the repository, and each part of a `module::Item` path or
//! a CamelCase `Type` is declared in a `.rs` file or `Cargo.toml`. Plain words in backticks stay prose.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const ITEM_KEYWORDS: &[&str] = &[
    "fn", "struct", "enum", "trait", "type", "const", "static", "mod",
];

#[test]
fn named_code_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = vec![root.join("Cargo.toml")];
    for dir in ["src", "tests"] {
        collect_source_files(&root.join(dir), &mut sources);
    }
    let mut paths = Vec::new();
    let mut declared = HashSet::new();
    for path in &sources {
        paths.push(path.to_string_lossy().replace('\\', "/"));
        declared.extend(
            path.strip_prefix(root)
                .expect("source under the root")
                .iter()
                .map(|part| part.to_string_lossy().trim_end_matches(".rs").to_string()),
        );
        // `name = "ghostmode"` in Cargo.toml declares the crate `ghostmode`.
        let keywords = if path.ends_with("Cargo.toml") {
            &["name"][..]
        } else {
            ITEM_KEYWORDS
        };
        let text = fs::read_to_string(path)
            .expect("read source file")
            .replace('-', "_");
        let words: Vec<&str> = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .collect();
        declared.extend(
            words
                .windows(2)
                .filter(|pair| keywords.contains(&pair[0]))
                .map(|pair| pair[1].to_string()),
        );
    }

    let mut docs = vec![root.join("AGENTS.md"), root.join("README.md")];
    if let Ok(entries) = fs::read_dir(root.join("docs")) {
        docs.extend(
            entries
                .map(|entry| entry.expect("docs/ entry").path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "md")),
        );
    }
    let mut missing = Vec::new();
    for doc in docs {
        let text = fs::read_to_string(&doc).expect("read doc");
        let mut in_fence = false;
        for (number, line) in text.lines().enumerate() {
            in_fence ^= line.trim_start().starts_with("```");
            for code in line.split('`').skip(1).step_by(2).filter(|_| !in_fence) {
                if let Some(problem) = describe_missing(code, &paths, &declared) {
                    let doc = doc.strip_prefix(root).unwrap_or(&doc).display();
                    missing.push(format!("{doc}:{}: `{code}` {problem}", number + 1));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "docs name code that does not exist:\n{}",
        missing.join("\n")
    );
}

/// Why `code` names nothing real, or `None` when it is real or is not a code name.
fn describe_missing(code: &str, paths: &[String], declared: &HashSet<String>) -> Option<String> {
    if code.ends_with(".rs") {
        let suffix = format!("/{code}");
        return (!paths.iter().any(|path| path.ends_with(&suffix)))
            .then(|| "is no file in the repository".to_string());
    }
    let code = code.strip_suffix("()").unwrap_or(code);
    let parts: Vec<&str> = code.split("::").collect();
    let is_identifier = |part: &&str| {
        part.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let is_camel_case = code.starts_with(|c: char| c.is_ascii_uppercase())
        && code[1..].starts_with(|c: char| c.is_ascii_lowercase());
    if !(parts.len() > 1 || is_camel_case) || !parts.iter().all(is_identifier) {
        return None;
    }
    let unknown: Vec<&str> = parts
        .into_iter()
        .filter(|part| !declared.contains(*part))
        .collect();
    (!unknown.is_empty()).then(|| format!("names {} that no source declares", unknown.join(", ")))
}

/// Every `.rs` file under `dir`.
fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source folder") {
        let path = entry.expect("source folder entry").path();
        if path.is_dir() {
            collect_source_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}
