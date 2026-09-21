//! Filters that preserve surrounding output and can compose inside shell batches.
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

static PNPM_PROGRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^Progress: resolved \d+, reused \d+, downloaded \d+, added \d+(?:, done)?$")
        .unwrap()
});
static PNPM_PACKAGES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:Packages: [+-]\d+(?: [+-]\d+)?|[+-]+)$").unwrap());
static YARN_STAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[\d+/\d+\] (?:Resolving packages|Fetching packages|Linking dependencies|Building fresh packages|Rebuilding all packages)\.\.\.$").unwrap()
});
static YARN_FETCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^➤ YN0013: │ [^\r\n]+ can't be found in the cache and will be fetched from (?:the remote registry|the disk)$").unwrap()
});
static YARN_PHASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^➤ YN0000: [┌└] (?:Resolution step|Post-resolution validation|Fetch step|Link step|Completed(?: in [\d.hms ]+)?)$").unwrap()
});
static RESTORED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^Restored [^\r\n]+\.(?:csproj|fsproj|vbproj) \(in [\d.,]+ (?:ms|sec|min)\)\.$")
        .unwrap()
});
static DIAGNOSTIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r": (?:warning|error) [A-Z]+\d+: ").unwrap());
static PREVIEW_NOTICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^.+: message NETSDK1057: (You are using a preview version of \.NET\. See: https://aka\.ms/dotnet-support-policy)(?: \[[^\r\n]+\])?$").unwrap()
});

fn progress_kind(line: &str) -> u8 {
    if PNPM_PROGRESS.is_match(line) {
        1
    } else if YARN_STAGE.is_match(line) || YARN_PHASE.is_match(line) {
        2
    } else if YARN_FETCH.is_match(line) {
        3
    } else if RESTORED.is_match(line) {
        4
    } else {
        0
    }
}

pub fn recognizes(input: &str) -> bool {
    crate::cmds::js::captured_output::recognizes(input)
        || input.lines().any(|line| {
            progress_kind(line.trim()) != 0
                || matches!(line.trim(), "Build succeeded." | "Build FAILED.")
        })
}

pub fn filter(input: &str) -> String {
    let javascript = crate::cmds::js::captured_output::filter(input);
    let mut lines = javascript.split_inclusive('\n').peekable();
    let mut output = String::with_capacity(javascript.len());
    let mut diagnostics = HashSet::new();
    let mut build_summary = false;
    let mut duplicates = 0;
    while let Some(line) = lines.next() {
        let text = line.trim();
        let kind = progress_kind(text);
        if kind != 0 {
            let mut count = 1;
            let mut last = line;
            // Only pnpm's package counts/bar may separate progress rows. Keep
            // that text, and never cross diagnostics or a completion boundary.
            while !(kind == 1 && last.trim().ends_with(", done")) {
                let mut candidate = lines.clone();
                let mut package_rows = String::new();
                if kind == 1 {
                    while candidate
                        .peek()
                        .is_some_and(|next| PNPM_PACKAGES.is_match(next.trim()))
                    {
                        package_rows.push_str(candidate.next().unwrap());
                    }
                }
                if !candidate
                    .peek()
                    .is_some_and(|next| progress_kind(next.trim()) == kind)
                {
                    break;
                }
                last = candidate.next().unwrap();
                lines = candidate;
                output.push_str(&package_rows);
                count += 1;
            }
            if count == 1 || kind == 1 || kind == 2 {
                output.push_str(last);
            } else if kind == 3 {
                output.push_str(&format!("[RTK: {count} Yarn cache-miss notices omitted]\n"));
            } else {
                output.push_str(&format!(
                    "Restored {count} projects [RTK: individual paths/timings omitted]\n"
                ));
            }
            continue;
        }
        if text == "Determining projects to restore..." || text.starts_with("Time Elapsed ") {
            if duplicates > 0 {
                output.push_str(&format!(
                    "[RTK: {duplicates} repeated build diagnostics omitted]\n"
                ));
                duplicates = 0;
            }
            diagnostics.clear();
            build_summary = false;
        }
        if text == "Determining projects to restore..."
            && lines
                .peek()
                .is_some_and(|next| next.trim() == "All projects are up-to-date for restore.")
        {
            continue;
        }
        // Suppress the known preview-SDK informational notice, including on failure.
        if PREVIEW_NOTICE.is_match(text) {
            continue;
        }
        if matches!(text, "Build succeeded." | "Build FAILED.") {
            build_summary = true;
        }
        if DIAGNOSTIC.is_match(text) {
            if build_summary && diagnostics.contains(text) {
                duplicates += 1;
                continue;
            }
            diagnostics.insert(text);
        }
        output.push_str(line);
    }
    if duplicates > 0 {
        output.push_str(&format!(
            "\n[RTK: {duplicates} repeated build diagnostics omitted]\n"
        ));
    }
    output
}
