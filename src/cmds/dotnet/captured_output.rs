//! Classic MSBuild/VSTest console output inside captured batches.
//!
//! Warnings are counted and passing rows dropped only when a completed build or test
//! outcome is beside them, so a `dotnet build | grep warning` the caller sliced on purpose
//! keeps every distinct diagnostic. What any output loses is repetition: a diagnostic
//! MSBuild printed twice, copy retries whose outcome follows, SDK and project paths
//! already shown. Errors, failed tests, stack traces and outcome lines are never removed.
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
static DIAGNOSTIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r": (?:warning|error) [A-Z][A-Za-z]{0,11}\d{2,6}: ").unwrap());
/// MSBuild retries a locked copy ten times and prints every attempt.
static COPY_RETRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#": warning MSB3026: Could not copy "([^"\r\n]+)" to "([^"\r\n]+)"\. Beginning retry \d+"#,
    )
    .unwrap()
});
/// A copy that ran out of retries; MSB3021 for the same file carries the reason.
static COPY_FAILED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#": error MSB3027: Could not copy "([^"\r\n]+)" to "([^"\r\n]+)"\. Exceeded retry count"#,
    )
    .unwrap()
});
static COPY_UNABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#": error MSB3021: Unable to copy file "([^"\r\n]+)" to "([^"\r\n]+)"\."#).unwrap()
});
/// `C:\Program Files\dotnet\sdk\10.0.100\Microsoft.Common.CurrentVersion.targets(5096,5): `
static SDK_TARGETS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^((?:\d+>)?)[^\r\n]*?[\\/]sdk[\\/][^\\/\r\n]+[\\/](?:[^\\/\r\n(]+[\\/])*([^\\/\r\n(]+\.targets\(\d+,\d+\): (?:warning|error) )").unwrap()
});
static PROJECT_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r" \[((?:[A-Za-z]:)?[\\/][^\]\r\n]*[\\/])([^\\/\]\r\n]+\.(?:cs|fs|vb)proj(?:::[^\]\r\n]*)?)\]$").unwrap()
});
/// `location(line,col): warning CODE: message [project]`, the location and project optional.
static WARNING_PARTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\d+>)?(.*?)(\(\d+(?:,\d+){0,3}\))?\s*: warning ([A-Z][A-Za-z]{0,11}\d{2,6}): (.*?)(?: \[([^\]\r\n]+)\])?$").unwrap()
});
/// `message, https://link`: the link differs per warning where the message does not.
static LINK_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.*?),? (https?://\S+)$").unwrap());
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
                || DIAGNOSTIC.is_match(text)
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
    let input = collapse_ef(&tidy_diagnostics(input));
    if !outcome {
        // No outcome: the caller may have sliced these on purpose, so every warning keeps
        // its location, and only the code and message are stated once per group.
        return group_warnings(&input);
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

/// Lossless-in-substance cleanup that also holds for sliced output: a diagnostic MSBuild
/// already printed, retries of a copy whose outcome follows, the SDK install path in front
/// of a targets file, and a project path the reader has already seen once.
fn tidy_diagnostics(input: &str) -> String {
    if !DIAGNOSTIC.is_match(input) {
        return input.to_string();
    }
    let unable: HashSet<(&str, &str)> = COPY_UNABLE
        .captures_iter(input)
        .map(|c| {
            let (_, [from, to]) = c.extract();
            (from, to)
        })
        .collect();
    let mut output = String::with_capacity(input.len());
    let mut seen: HashSet<String> = HashSet::new();
    let mut retried: HashSet<(&str, &str)> = HashSet::new();
    let mut projects: HashSet<String> = HashSet::new();
    let (mut repeats, mut retries, mut exhausted) = (0usize, 0usize, 0usize);
    for line in input.split_inclusive('\n') {
        let text = line.trim();
        if !DIAGNOSTIC.is_match(text) {
            output.push_str(line);
            continue;
        }
        if let Some(c) = COPY_RETRY.captures(text) {
            let (_, [from, to]) = c.extract();
            if !retried.insert((from, to)) {
                retries += 1;
                continue;
            }
        }
        if let Some(c) = COPY_FAILED.captures(text) {
            let (_, [from, to]) = c.extract();
            if unable.contains(&(from, to)) {
                exhausted += 1;
                continue;
            }
        }
        let short = SDK_TARGETS.replace(text, "$1$2");
        if !seen.insert(short.to_string()) {
            repeats += 1;
            continue;
        }
        let short = match PROJECT_SUFFIX.captures(&short) {
            Some(c) if !projects.insert(format!("{}{}", &c[1], &c[2])) => {
                let at = c.get(0).map_or(short.len(), |m| m.start());
                format!("{} [{}]", &short[..at], &c[2])
            }
            _ => short.into_owned(),
        };
        output.push_str(&line[..line.len() - line.trim_start().len()]);
        output.push_str(&short);
        output.push_str(&line[line.trim_end().len()..]);
    }
    let mut notes = Vec::new();
    if repeats > 0 {
        notes.push(format!(
            "[RTK: {repeats} repeated build diagnostics omitted]"
        ));
    }
    if retries > 0 {
        notes.push(format!(
            "[RTK: {retries} further MSB3026 copy retries of the same files omitted]"
        ));
    }
    if exhausted > 0 {
        notes.push(format!(
            "[RTK: {exhausted} MSB3027 lines omitted; the MSB3021 line for the same file gives the reason]"
        ));
    }
    if !notes.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        for note in notes {
            output.push_str(&note);
            output.push('\n');
        }
    }
    output
}

/// Where one warning of a group was reported, and the varying link its message ended with.
type Place<'a> = (String, Option<String>, Option<&'a str>);
type WarningGroup<'a> = ((&'a str, &'a str), Vec<Place<'a>>);

/// Replace three or more warning lines with one block at the first one's position:
/// warnings grouped by code and message, each group listing every location. The shared
/// directory and a single project are stated once. Other lines stay where they were.
fn group_warnings(input: &str) -> String {
    let mut groups: Vec<WarningGroup> = Vec::new();
    let mut output = String::with_capacity(input.len());
    let mut first = None;
    let mut count = 0usize;
    for line in input.split_inclusive('\n') {
        let Some(c) = WARNING_PARTS.captures(line.trim()) else {
            output.push_str(line);
            continue;
        };
        first.get_or_insert(output.len());
        count += 1;
        let (Some(code), Some(message)) = (c.get(3), c.get(4)) else {
            continue;
        };
        let location = format!(
            "{}{}",
            c.get(1).map_or("", |m| m.as_str().trim()),
            c.get(2).map_or("", |m| m.as_str())
        );
        let project = c.get(5).map(|m| project_name(m.as_str()));
        // NuGet ends each vulnerability with its own advisory link; group on the rest.
        let (message, link) = match LINK_TAIL.captures(message.as_str()) {
            Some(t) => (
                t.get(1).map_or("", |m| m.as_str()),
                t.get(2).map(|m| m.as_str()),
            ),
            None => (message.as_str(), None),
        };
        let key = (code.as_str(), message);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, at)) => at.push((location, project, link)),
            None => groups.push((key, vec![(location, project, link)])),
        }
    }
    let Some(first) = first.filter(|_| count >= 3) else {
        return input.to_string();
    };
    let located: Vec<&str> = groups
        .iter()
        .flat_map(|(_, at)| at.iter())
        .map(|(location, _, _)| location.as_str())
        .filter(|location| location.contains(['\\', '/']))
        .collect();
    let prefix = common_directory(&located);
    let projects: HashSet<&str> = groups
        .iter()
        .flat_map(|(_, at)| at.iter())
        .filter_map(|(_, project, _)| project.as_deref())
        .collect();
    let single_project = (projects.len() == 1)
        .then(|| projects.iter().next().copied())
        .flatten();
    let mut block = format!("dotnet: {count} warnings grouped by code and message");
    if !prefix.is_empty() {
        block.push_str(&format!("; paths under {prefix}"));
    }
    if let Some(project) = single_project {
        block.push_str(&format!("; project {project}"));
    }
    block.push_str(":\n");
    let place = |(location, project, _): &Place| {
        let location = location.strip_prefix(prefix).unwrap_or(location);
        let location = if location.is_empty() { "?" } else { location };
        match project {
            Some(project) if single_project.is_none() => format!("{location} [{project}]"),
            _ => location.to_string(),
        }
    };
    let linked = |at: &Place, text: String| match at.2 {
        Some(link) => format!("{text} {link}"),
        None => text,
    };
    for ((code, message), at) in &groups {
        let places: Vec<String> = at.iter().map(place).collect();
        if let [one] = at.as_slice() {
            block.push_str(&linked(one, format!("{}: {code}: {message}", places[0])));
            block.push('\n');
        } else if places.iter().all(|p| *p == places[0]) && at.iter().all(|a| a.2.is_some()) {
            // One location, several advisories: state the location once.
            let links: Vec<&str> = at.iter().filter_map(|a| a.2).collect();
            block.push_str(&format!(
                "{code} ×{} at {}: {message}\n  {}\n",
                at.len(),
                places[0],
                links.join(" ")
            ));
        } else {
            let listed: Vec<String> = at.iter().zip(places).map(|(a, p)| linked(a, p)).collect();
            block.push_str(&format!(
                "{code} ×{}: {message}\n  {}\n",
                at.len(),
                listed.join("; ")
            ));
        }
    }
    let first = first.min(output.len());
    if first > 0 && !output[..first].ends_with('\n') {
        block.insert(0, '\n');
    }
    output.insert_str(first, &block);
    if output.len() < input.len() {
        output
    } else {
        input.to_string()
    }
}

/// `C:\s\P\P.csproj::TargetFramework=net8.0` → `P.csproj::TargetFramework=net8.0`.
fn project_name(project: &str) -> String {
    let (path, rest) = project.split_once("::").unwrap_or((project, ""));
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    if rest.is_empty() {
        name.to_string()
    } else {
        format!("{name}::{rest}")
    }
}

/// The longest directory (ending in a separator) that every path starts with.
fn common_directory<'a>(paths: &[&'a str]) -> &'a str {
    let Some((head, rest)) = paths.split_first() else {
        return "";
    };
    if rest.is_empty() {
        return "";
    }
    let mut len = head.len();
    for path in rest {
        len = head
            .bytes()
            .zip(path.bytes())
            .take(len)
            .take_while(|(a, b)| a == b)
            .count();
    }
    while !head.is_char_boundary(len) {
        len -= 1;
    }
    match head[..len].rfind(['\\', '/']) {
        Some(end) if end >= 3 => &head[..=end],
        _ => "",
    }
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
        // The summary repeat of the error goes; the error itself stays.
        // The warnings above already named the project, so only its file name remains.
        assert_eq!(
            output
                .matches(
                    "C:\\s\\X.cs(9,9): error CS0103: The name 'Host' does not exist [P.csproj]\n"
                )
                .count(),
            1
        );
        assert!(output.contains("[RTK: 2 repeated build diagnostics omitted]"));
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
    fn sliced_warnings_group_by_code_and_keep_every_location() {
        let input = concat!(
            "C:\\s\\app\\src\\A.cs(1,2): warning CS8602: Dereference of a possibly null reference. [C:\\s\\app\\App.csproj]\n",
            "C:\\s\\app\\src\\B.cs(3,4): warning CS8602: Dereference of a possibly null reference. [C:\\s\\app\\App.csproj]\n",
            "C:\\s\\app\\src\\X.cs(9,9): error CS0103: The name 'Host' does not exist [C:\\s\\app\\App.csproj]\n",
            "C:\\s\\app\\src\\C.cs(5,6): warning CA1822: Member 'Go' does not access instance data. [C:\\s\\app\\App.csproj]\n",
            "C:\\s\\app\\src\\D.cs(7,8): warning CS8602: Dereference of a possibly null reference. [C:\\s\\app\\App.csproj]\n",
        );
        assert_eq!(
            filter(input),
            concat!(
                "dotnet: 4 warnings grouped by code and message; paths under C:\\s\\app\\src\\; project App.csproj:\n",
                "CS8602 ×3: Dereference of a possibly null reference.\n",
                "  A.cs(1,2); B.cs(3,4); D.cs(7,8)\n",
                "C.cs(5,6): CA1822: Member 'Go' does not access instance data.\n",
                "C:\\s\\app\\src\\X.cs(9,9): error CS0103: The name 'Host' does not exist [App.csproj]\n",
            )
        );
    }

    #[test]
    fn advisories_for_one_package_share_a_line() {
        let vuln = |level: &str, id: &str| {
            format!(
                "C:\\s\\Lib\\Lib.csproj : warning NU1903: Package 'Tpl' 5.1.0 has a known {level} severity vulnerability, https://github.com/advisories/{id} [C:\\s\\Lib\\Lib.csproj]\n"
            )
        };
        let input = format!(
            "{}{}{}",
            vuln("high", "GHSA-aaaa"),
            vuln("high", "GHSA-bbbb"),
            vuln("moderate", "GHSA-cccc")
        );
        assert_eq!(
            filter(&input),
            concat!(
                "dotnet: 3 warnings grouped by code and message; paths under C:\\s\\Lib\\; project Lib.csproj:\n",
                "NU1903 ×2 at Lib.csproj: Package 'Tpl' 5.1.0 has a known high severity vulnerability\n",
                "  https://github.com/advisories/GHSA-aaaa https://github.com/advisories/GHSA-bbbb\n",
                "Lib.csproj: NU1903: Package 'Tpl' 5.1.0 has a known moderate severity vulnerability https://github.com/advisories/GHSA-cccc\n",
            )
        );
    }

    #[test]
    fn linux_sdk_and_project_paths_shorten_like_windows_ones() {
        let targets =
            "/usr/share/dotnet/sdk/8.0.404/Microsoft.Common.CurrentVersion.targets(5096,5)";
        let input = format!(
            "{targets}: error MSB3021: Unable to copy file \"/app/obj/A.dll\" to \"bin/A.dll\". Access denied. [/app/A.csproj]\n{targets}: error MSB3021: Unable to copy file \"/app/obj/B.dll\" to \"bin/B.dll\". Access denied. [/app/A.csproj]\n"
        );
        assert_eq!(
            filter(&input),
            "Microsoft.Common.CurrentVersion.targets(5096,5): error MSB3021: Unable to copy file \"/app/obj/A.dll\" to \"bin/A.dll\". Access denied. [/app/A.csproj]\n\
Microsoft.Common.CurrentVersion.targets(5096,5): error MSB3021: Unable to copy file \"/app/obj/B.dll\" to \"bin/B.dll\". Access denied. [A.csproj]\n"
        );
    }

    #[test]
    fn grouped_warnings_name_projects_when_there_are_several() {
        let input = concat!(
            "/src/a/A.cs(1,2): warning CS0219: Unused 'x'. [/src/a/A.csproj::TargetFramework=net8.0]\n",
            "/src/a/A.cs(1,2): warning CS0219: Unused 'x'. [/src/a/A.csproj::TargetFramework=net472]\n",
            "/src/b/B.cs(4,4): warning CS0219: Unused 'x'. [/src/b/B.csproj]\n",
        );
        assert_eq!(
            filter(input),
            concat!(
                "dotnet: 3 warnings grouped by code and message; paths under /src/:\n",
                "CS0219 ×3: Unused 'x'.\n",
                "  a/A.cs(1,2) [A.csproj::TargetFramework=net8.0]; a/A.cs(1,2) [A.csproj::TargetFramework=net472]; b/B.cs(4,4) [B.csproj]\n",
            )
        );
    }

    #[test]
    fn sliced_diagnostics_lose_only_repetition() {
        let warn = "C:\\s\\A.cs(1,2): warning CS8602: Something. [C:\\s\\P\\P.csproj]\n";
        let other = "C:\\s\\B.cs(3,4): warning CS0219: Other. [C:\\s\\P\\P.csproj]\n";
        let input = format!("{warn}{other}{warn}");
        assert_eq!(
            filter(&input),
            format!(
                "{warn}C:\\s\\B.cs(3,4): warning CS0219: Other. [P.csproj]\n[RTK: 1 repeated build diagnostics omitted]\n"
            )
        );
    }

    #[test]
    fn locked_copies_keep_one_retry_and_the_reason() {
        let targets = "C:\\Program Files\\dotnet\\sdk\\10.0.100\\Microsoft.Common.CurrentVersion.targets(5096,5)";
        let retry = |n: u32| {
            format!(
                "{targets}: warning MSB3026: Could not copy \"C:\\s\\obj\\A.dll\" to \"bin\\A.dll\". Beginning retry {n} in 1000ms. The file is locked by: \"A (42)\" [C:\\s\\A.csproj]\n"
            )
        };
        let input = format!(
            "{}{}{}{targets}: error MSB3027: Could not copy \"C:\\s\\obj\\A.dll\" to \"bin\\A.dll\". Exceeded retry count of 10. Failed. [C:\\s\\A.csproj]\n{targets}: error MSB3021: Unable to copy file \"C:\\s\\obj\\A.dll\" to \"bin\\A.dll\". The process cannot access the file because it is being used by another process. [C:\\s\\A.csproj]\n",
            retry(1),
            retry(2),
            retry(3)
        );
        let output = filter(&input);
        assert_eq!(
            output,
            "Microsoft.Common.CurrentVersion.targets(5096,5): warning MSB3026: Could not copy \"C:\\s\\obj\\A.dll\" to \"bin\\A.dll\". Beginning retry 1 in 1000ms. The file is locked by: \"A (42)\" [C:\\s\\A.csproj]\n\
Microsoft.Common.CurrentVersion.targets(5096,5): error MSB3021: Unable to copy file \"C:\\s\\obj\\A.dll\" to \"bin\\A.dll\". The process cannot access the file because it is being used by another process. [A.csproj]\n\
[RTK: 2 further MSB3026 copy retries of the same files omitted]\n\
[RTK: 1 MSB3027 lines omitted; the MSB3021 line for the same file gives the reason]\n"
        );
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
