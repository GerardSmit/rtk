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
        r"|\(Use `[^`\r\n]+ --trace-(?:warnings|deprecation) \.\.\.` to show where the warning was created\)",
        r"|The Entity Framework tools version '[^'\r\n]+' is older than that of the runtime '[^'\r\n]+'\. Update the tools for the latest features and bug fixes\.[^\r\n]*)",
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

/// Line-shaped noise from tools that restate what did not change, one pattern per kind.
static ROUTINE: LazyLock<[(Regex, &str); 4]> = LazyLock::new(|| {
    [
        (
            Regex::new(r"^Requirement already satisfied: ").unwrap(),
            "pip \"Requirement already satisfied\" lines",
        ),
        (
            Regex::new(r"^(?:psql:[^\r\n]*?: )?NOTICE:\s+[^\r\n]* already exists, skipping$")
                .unwrap(),
            "Postgres \"already exists, skipping\" notices",
        ),
        (
            Regex::new(concat!(
                r"^#\d+ (?:DONE [\d.]+s|transferring [\w ]+: [^\r\n]*",
                r"|sha256:[0-9a-f]{64} [\d.]+\w?B / [\d.]+\w?B [^\r\n]*",
                r"|(?:extracting|resolve) [^\r\n]*sha256:[0-9a-f]{64}[^\r\n]*)$"
            ))
            .unwrap(),
            "Docker BuildKit progress lines",
        ),
        (
            // API self-links; `html_url` points at a page and stays.
            Regex::new(r#"^"\w+_url": "https://(?:api|uploads)\.github\.com/[^"\r\n]*",$"#)
                .unwrap(),
            "GitHub API *_url fields",
        ),
    ]
});
static SASS_DEPRECATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^DEPRECATION WARNING \[([\w-]+)\]: ").unwrap());
/// Lines of a Sass warning block after its headline: source excerpt, trace, link.
static SASS_BODY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[ \t]|\d+ │|More info(?: and automated migrator)?: https://sass-lang\.com/)")
        .unwrap()
});
static CARGO_OK_ROW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^test \S+(?: - [^\r\n]*)? \.\.\. ok$").unwrap());

fn routine_kind(text: &str) -> Option<usize> {
    ROUTINE
        .iter()
        .position(|(pattern, _)| pattern.is_match(text))
}

/// Drop routine lines from pip, psql, BuildKit and the GitHub API, repeated Sass
/// deprecation blocks after the first of each kind, and passing cargo test rows that a
/// `test result:` line accounts for. Each omission leaves a count.
fn routine_output(input: &str) -> String {
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut output = String::with_capacity(input.len());
    let mut counts = [0usize; 4];
    let mut sass: Vec<(&str, usize)> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let text = line.trim();
        index += 1;
        if let Some(kind) = routine_kind(text) {
            counts[kind] += 1;
            continue;
        }
        if let Some(kind) = SASS_DEPRECATION.captures(text).and_then(|c| c.get(1)) {
            let mut end = index;
            while end < lines.len()
                && (lines[end].trim().is_empty() || SASS_BODY.is_match(lines[end]))
                && !SASS_DEPRECATION.is_match(lines[end].trim())
            {
                end += 1;
            }
            match sass.iter_mut().find(|(seen, _)| *seen == kind.as_str()) {
                Some((_, more)) => {
                    *more += 1;
                    index = end;
                }
                None => {
                    sass.push((kind.as_str(), 0));
                    output.push_str(line);
                }
            }
            continue;
        }
        if CARGO_OK_ROW.is_match(text) {
            let mut end = index;
            while end < lines.len() && CARGO_OK_ROW.is_match(lines[end].trim()) {
                end += 1;
            }
            let mut after = end;
            while after < lines.len() && lines[after].trim().is_empty() {
                after += 1;
            }
            let rows = end - index + 1;
            if rows >= 3
                && lines
                    .get(after)
                    .is_some_and(|next| next.trim_start().starts_with("test result: "))
            {
                output.push_str(&format!("[RTK: {rows} passing test rows omitted]\n"));
                index = end;
                continue;
            }
        }
        output.push_str(line);
    }
    let mut notes: Vec<String> = counts
        .iter()
        .zip(ROUTINE.iter())
        .filter(|(count, _)| **count > 0)
        .map(|(count, (_, what))| format!("[RTK: {count} {what} omitted]"))
        .collect();
    let repeated: Vec<String> = sass
        .iter()
        .filter(|(_, more)| *more > 0)
        .map(|(kind, more)| format!("{kind} ×{more}"))
        .collect();
    if !repeated.is_empty() {
        notes.push(format!(
            "[RTK: repeated Sass deprecation warnings omitted ({})]",
            repeated.join(", ")
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
            let text = line.trim();
            progress_kind(text) != 0
                || matches!(text, "Build succeeded." | "Build FAILED.")
                || routine_kind(text).is_some()
                || SASS_DEPRECATION.is_match(text)
                || CARGO_OK_ROW.is_match(text)
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
    let clean = routine_output(&clean);
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
    fn routine_tool_lines_become_counts() {
        let input = concat!(
            "Requirement already satisfied: requests in ./venv/lib (2.32.3)\n",
            "Requirement already satisfied: idna in ./venv/lib (from requests) (3.7)\n",
            "Successfully installed widget-1.0\n",
            "NOTICE:  relation \"jobs\" already exists, skipping\n",
            "psql:up.sql:4: NOTICE:  column \"x\" of relation \"jobs\" already exists, skipping\n",
            "ERROR:  constraint \"jobs_pk\" for relation \"jobs\" already exists\n",
            "#7 [3/5] RUN npm ci\n",
            "#7 sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef 1.05MB / 3.2MB 0.4s\n",
            "#7 transferring context: 2.1kB done\n",
            "#7 DONE 12.3s\n",
            "{\n",
            "  \"url\": \"https://api.github.com/repos/o/r/releases/1\",\n",
            "  \"assets_url\": \"https://api.github.com/repos/o/r/releases/1/assets\",\n",
            "  \"html_url\": \"https://github.com/o/r/releases/tag/v1\",\n",
            "  \"upload_url\": \"https://uploads.github.com/repos/o/r/releases/1/assets{?name,label}\",\n",
            "  \"id\": 1\n",
            "}\n",
        );
        assert_eq!(
            filter(input),
            concat!(
                "Successfully installed widget-1.0\n",
                "ERROR:  constraint \"jobs_pk\" for relation \"jobs\" already exists\n",
                "#7 [3/5] RUN npm ci\n",
                "{\n",
                "  \"url\": \"https://api.github.com/repos/o/r/releases/1\",\n",
                "  \"html_url\": \"https://github.com/o/r/releases/tag/v1\",\n",
                "  \"id\": 1\n",
                "}\n",
                "[RTK: 2 pip \"Requirement already satisfied\" lines omitted]\n",
                "[RTK: 2 Postgres \"already exists, skipping\" notices omitted]\n",
                "[RTK: 3 Docker BuildKit progress lines omitted]\n",
                "[RTK: 2 GitHub API *_url fields omitted]\n",
            )
        );
    }

    #[test]
    fn repeated_sass_deprecations_keep_the_first_of_each_kind() {
        let block = |kind: &str, file: &str| {
            format!(
                "DEPRECATION WARNING [{kind}]: Sass @import rules are deprecated.\n\nMore info and automated migrator: https://sass-lang.com/d/import\n\n  ╷\n1 │ @import \"x\";\n  │         ^^^\n  ╵\n    {file} 1:9  root stylesheet\n\n"
            )
        };
        let input = format!(
            "{}{}{}✓ built in 2.1s\n",
            block("import", "a.scss"),
            block("import", "b.scss"),
            block("global-builtin", "c.scss")
        );
        assert_eq!(
            filter(&input),
            format!(
                "{}{}✓ built in 2.1s\n[RTK: repeated Sass deprecation warnings omitted (import ×1)]\n",
                block("import", "a.scss"),
                block("global-builtin", "c.scss")
            )
        );
    }

    #[test]
    fn passing_cargo_rows_go_only_beside_a_result() {
        let rows = "test a::one ... ok\ntest a::two ... ok\ntest a::three ... ok\n";
        assert_eq!(
            filter(&format!(
                "{rows}test a::four ... FAILED\n\n{rows}\ntest result: FAILED. 6 passed; 1 failed\n"
            )),
            "test a::one ... ok\ntest a::two ... ok\ntest a::three ... ok\ntest a::four ... FAILED\n\n[RTK: 3 passing test rows omitted]\n\ntest result: FAILED. 6 passed; 1 failed\n"
        );
        // A slice without the result line is the list the caller asked for.
        assert_eq!(filter(rows), rows);
    }

    #[test]
    fn ef_tools_version_notice_is_removed() {
        assert_eq!(
            filter(
                "The Entity Framework tools version '8.0.8' is older than that of the runtime '8.0.29'. Update the tools for the latest features and bug fixes. See https://aka.ms/AAc1fbw for more information.\nDone.\n"
            ),
            "Done.\n"
        );
    }

    #[test]
    fn rustfmt_verbatim_prefix_is_dropped() {
        assert_eq!(
            filter("Diff in \\\\?\\D:\\src\\main.rs:41:\n"),
            "Diff in D:\\src\\main.rs:41:\n"
        );
    }
}
