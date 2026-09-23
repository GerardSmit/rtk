//! Classic MSBuild/VSTest console output inside captured batches.
//!
//! Only text that has a completed build or test outcome beside it is touched, so a
//! `dotnet build | grep warning` the caller sliced on purpose stays verbatim. Errors,
//! failed tests, stack traces and outcome lines are never removed.
use regex::Regex;
use std::{collections::HashMap, collections::HashSet, sync::LazyLock};

static TEST_SUMMARY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(Passed|Failed|Skipped)!\s+- Failed:\s+(\d+), Passed:\s+(\d+), Skipped:\s+(\d+), Total:\s+(\d+), Duration: [^\r\n]+? - (\S+)(?: \(([^)\r\n]+)\))?$").unwrap()
});
/// `--logger "console;verbosity=detailed"` ends with this instead of `Passed!` lines.
static DETAILED_OUTCOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^Test Run (?:Successful|Failed|Aborted)\.\r?\nTotal tests: \d+").unwrap()
});
static BUILD_OUTCOME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Build succeeded\.|Build FAILED\.|\d+ Warning\(s\)|Build (?:succeeded|failed) with \d+ (?:warning|error)\(s\) in [\d.]+s)$").unwrap()
});
static BANNER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:Test run for [^\r\n]+\.(?:dll|exe) \([^)\r\n]+\)",
        r"|A total of \d+ test files? matched the specified pattern\.",
        r"|VSTest version [\d.]+[^\r\n]*",
        r"|Microsoft \(R\) Test Execution Command Line Tool Version [^\r\n]+",
        r"|Copyright \(c\) Microsoft Corporation\.\s+All rights reserved\.",
        r"|Starting test execution, please wait\.\.\.)$"
    ))
    .unwrap()
});
static XUNIT_PROGRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[xUnit\.net [\d:.]+\]\s+(?:Discovering|Discovered|Starting|Finished):\s")
        .unwrap()
});
static PASSED_ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Passed \S[^\r\n]* \[(?:< ?)?[\d.]+ ?(?:ms|s|m)\]$").unwrap());
static OUTPUT_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\w.\-]+ -> (?:[A-Za-z]:[\\/]|/)[^\r\n]*\.(?:dll|exe)$").unwrap()
});
static WARNING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\d+>)?\S[^\r\n]*?(?:\(\d+(?:,\d+){0,3}\))?\s*: warning ([A-Z][A-Za-z]{0,11}\d{2,6}): ").unwrap()
});
/// EF Core model-validation messages repeated once per entity or property.
static EF_TEMPLATES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^Entity '[^']+' has a global query filter defined and is the required end of a relationship with the entity '[^']+'\. ").unwrap(),
        Regex::new(r"^The '[^']+' property '[^']+' on entity type '[^']+' is configured with a database-generated default, but has no configured sentinel value\. ").unwrap(),
    ]
});

pub fn recognizes(input: &str) -> bool {
    DETAILED_OUTCOME.is_match(input)
        || input.lines().any(|line| {
            let text = line.trim();
            TEST_SUMMARY.is_match(text)
                || BUILD_OUTCOME.is_match(text)
                || ef_template(text).is_some()
        })
}

fn ef_template(text: &str) -> Option<usize> {
    EF_TEMPLATES
        .iter()
        .position(|template| template.is_match(text))
}

pub fn filter(input: &str) -> String {
    let tests = DETAILED_OUTCOME.is_match(input)
        || input.lines().any(|line| TEST_SUMMARY.is_match(line.trim()));
    let outcome = tests
        || input
            .lines()
            .any(|line| BUILD_OUTCOME.is_match(line.trim()));
    let input = collapse_ef(input);
    if !outcome {
        return input;
    }
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut output = String::with_capacity(input.len());
    let mut passed_rows = 0usize;
    let mut paths = 0usize;
    let mut warnings: HashMap<&str, usize> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut warning_total = 0usize;
    let mut warning_at = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let text = line.trim();
        index += 1;
        if tests && (BANNER.is_match(text) || XUNIT_PROGRESS.is_match(text)) {
            continue;
        }
        if tests && PASSED_ROW.is_match(text) {
            passed_rows += 1;
            continue;
        }
        if OUTPUT_PATH.is_match(text) {
            paths += 1;
            continue;
        }
        if let Some(code) = WARNING.captures(text).and_then(|c| c.get(1)) {
            // The final summary repeats each warning; count a warning once.
            if seen.insert(text) {
                *warnings.entry(code.as_str()).or_default() += 1;
                warning_total += 1;
            }
            warning_at.get_or_insert(output.len());
            continue;
        }
        if let Some(summary) = passed_summary(text) {
            // Aggregate a run of per-assembly `Passed!` lines; blank lines may separate them.
            let mut run = vec![summary];
            let mut next = index;
            while next < lines.len() {
                let candidate = lines[next].trim();
                if candidate.is_empty() {
                    next += 1;
                    continue;
                }
                match passed_summary(candidate) {
                    Some(summary) => {
                        run.push(summary);
                        next += 1;
                        index = next;
                    }
                    None => break,
                }
            }
            if run.len() == 1 {
                output.push_str(line);
            } else {
                let sum = |field: usize| run.iter().map(|s| s[field]).sum::<u64>();
                output.push_str(&format!(
                    "Passed!  - Failed:     0, Passed: {}, Skipped: {}, Total: {} across {} test assemblies [RTK: per-assembly rows omitted]\n",
                    sum(1),
                    sum(2),
                    sum(3),
                    run.len()
                ));
            }
            continue;
        }
        output.push_str(line);
    }
    let mut notes = Vec::new();
    if passed_rows > 0 {
        notes.push(format!("[RTK: {passed_rows} passing test rows omitted]\n"));
    }
    if paths > 0 {
        notes.push(format!("[RTK: {paths} project output paths omitted]\n"));
    }
    if let Some(at) = warning_at {
        let mut codes: Vec<_> = warnings.into_iter().collect();
        codes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let mut listed: Vec<String> = codes
            .iter()
            .take(6)
            .map(|(code, count)| format!("{code} ×{count}"))
            .collect();
        if codes.len() > 6 {
            listed.push(format!("+{} more codes", codes.len() - 6));
        }
        let noun = if warning_total == 1 {
            "warning"
        } else {
            "warnings"
        };
        let summary = format!(
            "dotnet: {warning_total} {noun} ({}; details in full output).\n",
            listed.join(", ")
        );
        let at = at.min(output.len());
        if at > 0 && !output[..at].ends_with('\n') {
            output.insert(at, '\n');
            output.insert_str(at + 1, &summary);
        } else {
            output.insert_str(at, &summary);
        }
    }
    if !notes.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        for note in notes {
            output.push_str(&note);
        }
    }
    output
}

/// `[failed, passed, skipped, total]` for a successful per-assembly summary.
fn passed_summary(text: &str) -> Option<[u64; 4]> {
    let captures = TEST_SUMMARY.captures(text)?;
    if &captures[1] != "Passed" || &captures[2] != "0" {
        return None;
    }
    let field = |i: usize| captures[i].parse::<u64>().ok();
    Some([0, field(3)?, field(4)?, field(5)?])
}

fn collapse_ef(input: &str) -> String {
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let Some(template) = ef_template(line.trim()) else {
            output.push_str(line);
            continue;
        };
        let mut more = 0;
        while index < lines.len() && ef_template(lines[index].trim()) == Some(template) {
            more += 1;
            index += 1;
        }
        output.push_str(line);
        if more > 0 {
            if !line.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(&format!(
                "[RTK: {more} more EF Core warnings of the same kind omitted]\n"
            ));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed(n: u32, name: &str) -> String {
        format!(
            "Passed!  - Failed:     0, Passed:   {n}, Skipped:     0, Total:   {n}, Duration: 1 s - {name} (net10.0)\n"
        )
    }

    #[test]
    fn successful_test_run_keeps_only_totals() {
        let input = format!(
            "Test run for C:\\s\\A.Tests\\bin\\Debug\\net10.0\\A.Tests.dll (.NETCoreApp,Version=v10.0)\nA total of 1 test files matched the specified pattern.\n  A -> C:\\s\\A\\bin\\Debug\\net10.0\\A.dll\n  Passed A.Tests.One [4 ms]\n  Passed A.Tests.Two [< 1 ms]\n{}\n{}",
            passed(340, "A.Tests.dll"),
            passed(15, "B.Tests.dll")
        );
        let output = filter(&input);
        assert_eq!(
            output,
            "Passed!  - Failed:     0, Passed: 355, Skipped: 0, Total: 355 across 2 test assemblies [RTK: per-assembly rows omitted]\n[RTK: 2 passing test rows omitted]\n[RTK: 1 project output paths omitted]\n"
        );
    }

    #[test]
    fn failures_stay_verbatim() {
        let failed = "Failed!  - Failed:     1, Passed:  1858, Skipped:     0, Total:  1859, Duration: 1 m 44 s - Sample.Tests.dll (net10.0)\n";
        let input = format!(
            "{}  Failed A.Tests.Three [5 ms]\n  Error Message:\n   Assert.Equal() Failure\n{failed}{}",
            passed(3, "A.dll"),
            passed(4, "B.dll")
        );
        let output = filter(&input);
        assert!(output.contains(
            "  Failed A.Tests.Three [5 ms]\n  Error Message:\n   Assert.Equal() Failure\n"
        ));
        assert!(output.contains(failed));
        assert!(
            output.contains("Passed:   3,"),
            "a lone Passed! stays: {output}"
        );
    }

    #[test]
    fn warnings_become_counts_by_code_and_errors_stay() {
        let warn = |file: &str, code: &str| {
            format!("C:\\s\\{file}.cs(1,2): warning {code}: Something. [C:\\s\\P.csproj]\n")
        };
        let error =
            "C:\\s\\X.cs(9,9): error CS0103: The name 'Host' does not exist [C:\\s\\P.csproj]\n";
        let input = format!(
            "{}{}{}C:\\s\\P.csproj : warning NU1903: Package 'X' 1.0.0 has a known high severity vulnerability\n{error}\nBuild FAILED.\n\n{}{error}    3 Warning(s)\n    1 Error(s)\n",
            warn("A", "CS8602"),
            warn("B", "CS8602"),
            warn("C", "CA1822"),
            warn("A", "CS8602"),
        );
        let output = filter(&input);
        assert!(
            output.starts_with(
                "dotnet: 4 warnings (CS8602 ×2, CA1822 ×1, NU1903 ×1; details in full output).\n"
            ),
            "{output}"
        );
        assert_eq!(output.matches(error).count(), 2);
        assert!(output.contains("Build FAILED.") && output.contains("3 Warning(s)"));
        assert!(!output.contains(": warning "));
    }

    #[test]
    fn detailed_logger_outcome_counts_as_a_test_run() {
        let input = "  Passed A.One [3 s]\n  Failed A.Two [48 ms]\n  Error Message:\n   boom\n\nTest Run Failed.\nTotal tests: 2\n     Passed: 1\n     Failed: 1\n";
        let output = filter(input);
        assert!(!output.contains("Passed A.One"), "{output}");
        assert!(output.contains("  Failed A.Two [48 ms]\n  Error Message:\n   boom\n"));
        assert!(output.contains("Test Run Failed.\nTotal tests: 2\n     Passed: 1\n"));
        // Without the outcome block the rows were sliced on purpose and stay.
        let sliced = "  Passed A.One [3 s]\n";
        assert_eq!(filter(sliced), sliced);
    }

    #[test]
    fn sliced_warning_lists_stay_raw() {
        let input = "C:\\s\\A.cs(1,2): warning CS8602: Something. [C:\\s\\P.csproj]\n";
        assert_eq!(filter(input), input);
    }

    #[test]
    fn repeated_ef_warnings_keep_the_first() {
        let line = |a: &str, b: &str| {
            format!(
                "Entity '{a}' has a global query filter defined and is the required end of a relationship with the entity '{b}'. This may lead to unexpected results.\n"
            )
        };
        let input = format!(
            "{}{}{}Done.\n",
            line("A", "B"),
            line("A", "C"),
            line("D", "E")
        );
        assert_eq!(
            filter(&input),
            format!(
                "{}[RTK: 2 more EF Core warnings of the same kind omitted]\nDone.\n",
                line("A", "B")
            )
        );
    }
}
