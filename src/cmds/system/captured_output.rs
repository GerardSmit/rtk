//! Filters that preserve surrounding output and can compose inside shell batches.
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

static SPINNERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]+").unwrap());
static FUNDING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:\d+ packages? (?:are|is) looking for funding|run `npm fund` for details)[ \t]*\r?(?:\n|$)").unwrap()
});
// Notices that ride along with real output and never change what an agent does next.
static NOTICES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?m)^[ \t]*(?:npm warn Unknown (?:user|env|project|builtin|global) config "[^"\r\n]+"\. "#,
        r"This will stop working in the next major version of npm\.[^\r\n]*",
        r"|\(Use `[^`\r\n]+ --trace-(?:warnings|deprecation) \.\.\.` to show where the warning was created\))",
        r"[ \t]*\r?(?:\n|$)"
    ))
    .unwrap()
});
// A failed link prints the whole linker invocation: tens of KB of object paths.
static LINKER_COMMAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^([ \t]*= note: )"(?:[^"\r\n]*[\\/])?([^"\\/\r\n]+)" [^\r\n]{400,}$"#)
        .unwrap()
});
// rustfmt reports Windows paths in their verbatim `\\?\` form.
static FMT_VERBATIM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Diff in \\\\\?\\").unwrap());
static CARGO_PROGRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:Compiling|Checking|Fresh) [\w-]+ v\d[^\r\n]*(?:\r?\n|$)").unwrap()
});
static CARGO_FINISHED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^[ \t]*Finished `([^`\r\n]+)` profile \[[^\]\r\n]+\] target\(s\) in ([^\r\n]+)",
    )
    .unwrap()
});
static CARGO_WARNING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^warning(?:\[[^\]\r\n]+\])?:[^\r\n]*\r?\n[ \t]+-->").unwrap()
});
static WARNING_BODY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[ \t]*(?:-->|:::|\d+[ \t]*\||\||= (?:note|help):|help:|note:|\.\.\.$)").unwrap()
});
static WARNING_TOTAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^warning: `[^`]+`.* generated \d+ warnings?\b").unwrap());

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
    CARGO_PROGRESS.is_match(input)
        || CARGO_FINISHED.is_match(input)
        || CARGO_WARNING.is_match(input)
        || SPINNERS.is_match(input)
        || FUNDING.is_match(input)
        || NOTICES.is_match(input)
        || LINKER_COMMAND.is_match(input)
        || FMT_VERBATIM.is_match(input)
        || crate::cmds::js::captured_output::recognizes(input)
        || crate::cmds::git::captured_output::recognizes(input)
        || crate::cmds::dotnet::captured_output::recognizes(input)
        || input.lines().any(|line| {
            progress_kind(line.trim()) != 0
                || matches!(line.trim(), "Build succeeded." | "Build FAILED.")
        })
}

pub fn filter(input: &str) -> String {
    // Clean progress noise before format detection: spinner frames can be joined
    // directly to real warnings, summaries and command banners.
    let clean = SPINNERS.replace_all(input, "");
    let clean = FUNDING.replace_all(&clean, "");
    let clean = NOTICES.replace_all(&clean, "");
    let clean = LINKER_COMMAND.replace_all(&clean, |c: &regex::Captures| {
        let omitted = c[0].len() - c[1].len();
        format!(
            "{}\"{}\" [RTK: {omitted}-byte linker command line omitted]",
            &c[1], &c[2]
        )
    });
    let clean = FMT_VERBATIM.replace_all(&clean, "Diff in ");
    let cargo = cargo_compilation(&clean);
    let git = crate::cmds::git::captured_output::filter(&cargo);
    let javascript = crate::cmds::js::captured_output::filter(&git);
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
    crate::cmds::dotnet::captured_output::filter(&output)
}

fn cargo_compilation(input: &str) -> String {
    if !CARGO_PROGRESS.is_match(input)
        && !CARGO_FINISHED.is_match(input)
        && !CARGO_WARNING.is_match(input)
    {
        return input.to_string();
    }
    let clean = CARGO_PROGRESS.replace_all(input, "");
    let clean = CARGO_FINISHED.replace_all(&clean, "Cargo finished ($1, $2)");
    let mut output = String::with_capacity(clean.len());
    let mut warning = false;
    let mut warnings = 0usize;
    let mut lines = clean.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let text = line.trim();
        if warning && (text.is_empty() || WARNING_BODY.is_match(text)) {
            continue;
        }
        warning = false;
        // Only hide rendered rustc diagnostics with a source-location line.
        // Cargo/build-script warnings without that structure remain visible.
        if (text.starts_with("warning:") || text.starts_with("warning["))
            && lines
                .peek()
                .is_some_and(|next| next.trim_start().starts_with("-->"))
        {
            warning = true;
            warnings += 1;
            continue;
        }
        if warnings > 0 && (text.is_empty() || WARNING_TOTAL.is_match(text)) {
            continue;
        }
        if warnings > 0 {
            output.push_str(&format!(
                "Cargo: {warnings} warnings (details in full output).\n"
            ));
            warnings = 0;
        }
        output.push_str(line);
    }
    if warnings > 0 {
        output.push_str(&format!(
            "Cargo: {warnings} warnings (details in full output).\n"
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_are_removed() {
        let input = "npm warn Unknown user config \"python\". This will stop working in the next major version of npm. See `npm help npmrc` for supported config options.\n(node:1) Warning: something real\n(Use `node --trace-warnings ...` to show where the warning was created)\nok\n";
        assert_eq!(filter(input), "(node:1) Warning: something real\nok\n");
    }

    #[test]
    fn linker_command_is_shortened_and_error_kept() {
        // rustc prints the invocation Debug-escaped, so paths carry doubled backslashes.
        let objects = r#""C:\\t\\deps\\x.o" "#.repeat(40);
        let input = format!(
            "error: linking with `link.exe` failed: exit code: 1104\n  |\n  = note: \"D:\\\\VS\\\\bin\\\\link.exe\" \"/NOLOGO\" {objects}\n  = note: LINK : fatal error LNK1104: cannot open file 'x.exe'\n"
        );
        let output = filter(&input);
        assert!(output.contains("  = note: \"link.exe\" [RTK: "), "{output}");
        assert!(output.contains("-byte linker command line omitted]\n"));
        assert!(output.contains("fatal error LNK1104: cannot open file 'x.exe'"));
        assert!(output.len() < input.len() / 2);
    }

    #[test]
    fn rustfmt_verbatim_prefix_is_dropped() {
        assert_eq!(
            filter("Diff in \\\\?\\D:\\src\\main.rs:41:\n"),
            "Diff in D:\\src\\main.rs:41:\n"
        );
    }
}
