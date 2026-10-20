//! Repository contracts: the fuzz corpus, the release and coverage gates, the
//! install instructions, and the text streams the binary writes.

mod common;

use std::io::Write;

use common::*;
use pacioliscube::TextStream;
use pacioliscube::error::{Error, Uncaught};
use pacioliscube::fuzz::{check_input, check_with};
use pacioliscube::rules::parse_rules;

fn read(relative: &str) -> String {
    std::fs::read_to_string(repo().join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}

// ---------------------------------------------------------------- fuzzing

#[test]
fn the_seed_corpus_replays_on_every_platform() {
    let mut seeds: Vec<_> = std::fs::read_dir(repo().join("fuzz").join("corpus"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    seeds.sort();
    assert!(seeds.len() >= 8);
    for seed in seeds {
        let data = std::fs::read(&seed).unwrap();
        let outcome = big_stack(move || check_input(&data));
        assert_eq!(outcome, Ok(()), "{seed:?}");
    }
}

#[test]
fn unexpected_parser_errors_and_changing_answers_fail_the_check() {
    let failing = |_: &str| Err(Error::Uncaught(Uncaught::new("RuntimeError", "bug")));
    assert!(
        check_with(b"['Amount'] = 1;", failing)
            .unwrap_err()
            .contains("bug")
    );
    let calls = std::cell::Cell::new(0);
    let changing = |_: &str| {
        calls.set(calls.get() + 1);
        let text = if calls.get() == 1 {
            "['A'] = 1;"
        } else {
            "['A'] = 2;"
        };
        parse_rules(text, "Fuzz.rules")
    };
    assert!(
        check_with(b"['Amount'] = 1;", changing)
            .unwrap_err()
            .contains("different results")
    );
    assert_eq!(
        check_with(b"['A' = 1;", |t| parse_rules(t, "Fuzz.rules")),
        Ok(())
    );
}

// ---------------------------------------------------------------- release

const REQUIRED: [&str; 9] = [
    ".github/workflows/no-ai-attribution.yml: Attribution policy / Attribution policy runner",
    ".github/workflows/ci.yml: dependency-audit",
    ".github/workflows/ci.yml: lint",
    ".github/workflows/ci.yml: package",
    ".github/workflows/ci.yml: test (ubuntu-latest)",
    ".github/workflows/ci.yml: test (windows-latest)",
    ".github/workflows/ci.yml: tests-gates",
    ".github/workflows/codeql.yml: analyse",
    ".github/workflows/codeql.yml: codeql-gates",
];

#[test]
fn every_release_caller_requires_its_component_checks() {
    let workflows = repo().join(".github").join("workflows");
    let mut callers: Vec<String> = std::fs::read_dir(&workflows)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("release") && (n.ends_with(".yml") || n.ends_with(".yaml")))
        .collect();
    callers.sort();
    assert_eq!(callers, ["release.yml"]);
    let text = read(".github/workflows/release.yml");
    let job = text.split("\n  release:\n").nth(1).unwrap();
    // The pin is the caller's to set; a re-pin edits release.yml and nothing here.
    let pin = job
        .lines()
        .find_map(|line| {
            line.strip_prefix(
                "    uses: ryanduguid/release-policy/.github/workflows/release-archive.yml@",
            )
        })
        .expect("the release must call the source-archive workflow");
    assert!(
        pin.len() == 40 && pin.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "the source-archive workflow must be pinned by full commit, not {pin:?}"
    );
    let permissions = job
        .split("    permissions:\n")
        .nth(1)
        .unwrap()
        .split("    uses:")
        .next()
        .unwrap();
    assert!(
        permissions
            .lines()
            .any(|l| l.trim_start().starts_with("actions: read"))
    );
    let checks = job.split("      required-checks: |\n").nth(1).unwrap();
    let actual: Vec<&str> = checks
        .lines()
        .take_while(|l| l.starts_with("        "))
        .map(str::trim)
        .collect();
    let mut sorted_actual = actual.clone();
    sorted_actual.sort();
    let mut expected = REQUIRED.to_vec();
    expected.sort();
    assert_eq!(sorted_actual, expected);
    let mut unique = actual.clone();
    unique.dedup();
    assert_eq!(unique.len(), actual.len(), "duplicate check selector");
    for selector in actual {
        let (path, name) = selector.split_once(": ").unwrap();
        assert!(repo().join(path).is_file(), "{path}");
        assert!(!name.ends_with(" / gates"), "{name}");
    }
}

#[test]
fn the_version_file_matches_the_crate_and_its_lock() {
    let version = read("VERSION").trim().to_string();
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    assert!(read("Cargo.lock").contains(&format!(
        "name = \"pacioliscube\"\nversion = \"{version}\"\n"
    )));
}

// ---------------------------------------------------------------- CI gates

#[test]
fn line_coverage_is_measured_and_gated() {
    let workflow = read(".github/workflows/ci.yml");
    assert!(workflow.contains("cargo install cargo-llvm-cov --locked --version "));
    assert!(workflow.contains(
        "cargo llvm-cov --locked --lcov --output-path coverage-codecov.lcov --fail-under-lines 90"
    ));
    // The upload holds the OIDC token, so it runs apart from the tests.
    let upload = workflow
        .split("\n  codecov:\n")
        .nth(1)
        .unwrap()
        .split("\n  dependency-audit:\n")
        .next()
        .unwrap();
    assert!(upload.contains("id-token: write") && !upload.contains("cargo "));
}

/// A job holding a write permission must not build or run this repository's
/// code; the binary it needs comes from a separate read-only job.
#[test]
fn no_job_that_can_write_runs_repository_code() {
    for entry in std::fs::read_dir(repo().join(".github").join("workflows")).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let Some((_, jobs)) = text.split_once("\njobs:\n") else {
            continue;
        };
        let mut blocks: Vec<String> = Vec::new();
        for line in jobs.lines() {
            if line.starts_with("  ")
                && !line.starts_with("   ")
                && !line.trim_start().starts_with('#')
            {
                blocks.push(String::new());
            }
            if let Some(block) = blocks.last_mut() {
                block.push_str(line);
                block.push('\n');
            }
        }
        for block in blocks {
            let writes = block.lines().any(|l| {
                let l = l.trim();
                l.ends_with(": write") || l.contains(": write ") || l.contains(": write#")
            });
            if writes {
                for forbidden in [
                    "cargo ",
                    "pacioliscube ",
                    "target/",
                    "github/codeql-action/init@",
                    "github/codeql-action/analyze@",
                ] {
                    assert!(
                        !block.contains(forbidden),
                        "{path:?}: a writing job runs `{forbidden}`:\n{block}"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------- install instructions

#[test]
fn the_readme_declares_how_the_engine_is_installed() {
    let readme = read("README.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(readme.contains("**Package lifecycle:**"));
    assert!(readme.contains("cargo install --locked --path ."));
    assert!(
        readme.contains("git clone https://github.com/ryanduguid/planning-analytics-model.git")
    );
    assert!(
        readme.contains("0.3.0"),
        "the README names the last Python release"
    );
    assert!(!readme.contains("python -m pip install ."));
}

// ---------------------------------------------------------------- text streams

#[test]
fn text_streams_end_lines_as_python_did_on_this_platform() {
    let mut stream = TextStream::new(Vec::new());
    write!(stream, "a\nb\n\nc").unwrap();
    stream.flush().unwrap();
    let bytes = stream.into_inner();
    if cfg!(windows) {
        assert_eq!(bytes, b"a\r\nb\r\n\r\nc");
    } else {
        assert_eq!(bytes, b"a\nb\n\nc");
    }
}

#[test]
fn the_binary_writes_python_line_endings() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .args(["validate", &s(&model_root())])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let expected: &[u8] = if cfg!(windows) {
        b"0 errors, 0 warnings\r\n"
    } else {
        b"0 errors, 0 warnings\n"
    };
    assert_eq!(output.stdout, expected);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
        .arg("bogus")
        .env_remove("FORCE_COLOR")
        .env_remove("PYTHON_COLORS")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(err.starts_with("usage: pacioliscube [-h] [--version] SUBCOMMAND ..."));
    assert_eq!(err.contains("\r\n"), cfg!(windows));
}
