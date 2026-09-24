use rtk_filter::{MAX_INPUT_BYTES, filter, supports_filter};
use std::borrow::Cow;

#[test]
fn filters_cargo_without_a_process() {
    let raw = "running 2 tests\ntest first ... ok\ntest second ... ok\n\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
    let result = filter("cargo-test", raw).unwrap();
    assert!(matches!(result, Cow::Owned(_)));
    assert!(result.contains("2 passed"), "{result}");
    assert!(result.len() < raw.len());
    assert_eq!(filter("cargo", raw), Some(result));
}

#[test]
fn keeps_failure_details() {
    let raw = "running 1 test\ntest important ... FAILED\n\nfailures:\n\n---- important stdout ----\nassertion failed: expected 42\n\nfailures:\n    important\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n";
    let result = filter("cargo-test", raw).unwrap();
    assert!(result.contains("important"), "{result}");
    assert!(result.contains("expected 42"), "{result}");
}

#[test]
fn unknown_names_are_explicit() {
    assert!(!supports_filter("not-a-filter"));
    assert_eq!(filter("not-a-filter", "data"), None);
}

#[test]
fn oversized_and_empty_inputs_are_borrowed() {
    let oversized = "x".repeat(MAX_INPUT_BYTES + 1);
    for input in ["", oversized.as_str()] {
        assert!(matches!(filter("cargo-test", input), Some(Cow::Borrowed(raw)) if raw == input));
    }
}

#[test]
fn empty_or_larger_results_keep_raw_text() {
    for (name, input) in [
        ("cargo-test", "x"),
        ("git-status", ""),
        ("ruff-check", "[]"),
    ] {
        assert!(matches!(filter(name, input), Some(Cow::Borrowed(raw)) if raw == input));
    }
}

#[test]
fn every_filter_accepts_unicode_without_losing_all_output() {
    let names = [
        "cargo-test",
        "pytest",
        "go-test",
        "go-build",
        "ctest",
        "tsc",
        "vitest",
        "grep",
        "rg",
        "find",
        "fd",
        "git-log",
        "git-diff",
        "git-status",
        "log",
        "mypy",
        "ruff-check",
        "ruff-format",
        "sqlfluff-lint",
        "prettier",
        "phpunit",
        "pest",
        "paratest",
        "php-test",
        "ecs",
        "phpstan",
        "pint",
    ];
    let input = "unrecognized 🦀 output\n東京 café\n";
    for name in names {
        assert!(supports_filter(name), "{name}");
        let output = filter(name, input).unwrap();
        assert!(!output.is_empty(), "{name}");
        assert!(output.len() <= input.len(), "{name}");
    }
}
