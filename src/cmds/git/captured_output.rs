//! Git notices that ride along with real output, removed wherever they appear in a batch.
use regex::Regex;
use std::sync::LazyLock;

static LINE_ENDING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:warning: in the working copy of '[^\r\n]+', (?:LF|CRLF) will be replaced by (?:CRLF|LF) the next time Git touches it",
        r"|warning: (?:LF|CRLF) will be replaced by (?:CRLF|LF) in [^\r\n]+\.",
        r"|The file will have its original line endings in your working directory)$"
    ))
    .unwrap()
});
static DIFF_HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^diff --git \S").unwrap());
static MODE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:old mode|new mode|new file mode|deleted file mode) \d{6}$").unwrap()
});
static INDEX_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^index [0-9a-f]{7,64}\.\.[0-9a-f]{7,64}(?: \d{6})?$").unwrap());
static STATUS_SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:Changes not staged for commit|Changes to be committed|Untracked files|Unmerged paths):\r?$")
        .unwrap()
});
static STATUS_HELP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\(use "git [^"\r\n]+"[^\r\n]*\)$"#).unwrap());

pub fn recognizes(input: &str) -> bool {
    input
        .lines()
        .any(|line| LINE_ENDING.is_match(line.trim_end()))
        || STATUS_SECTION.is_match(input)
        || input.contains("\ndiff --git ")
        || input.starts_with("diff --git ")
}

pub fn filter(input: &str) -> String {
    let (input, eol_files, eol_lines) = line_ending_only_changes(input);
    let input = input.as_str();
    let status = STATUS_SECTION.is_match(input);
    let mut output = String::with_capacity(input.len());
    let mut line_endings = 0usize;
    // Inside a diff header: after `diff --git` and before the first `---`/`@@`.
    let mut header = false;
    for line in input.split_inclusive('\n') {
        let text = line.trim_end();
        if LINE_ENDING.is_match(text) {
            line_endings += 1;
            continue;
        }
        if DIFF_HEADER.is_match(text) {
            header = true;
        } else if header && INDEX_LINE.is_match(text) {
            // Blob ids and an unchanged mode say nothing a reader of the hunk needs.
            header = false;
            continue;
        } else if !(header && MODE_LINE.is_match(text)) {
            header = false;
        }
        if status && STATUS_HELP.is_match(text.trim_start()) {
            continue;
        }
        output.push_str(line);
    }
    if line_endings > 0 {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&format!(
            "[RTK: {line_endings} Git line-ending (LF/CRLF) warnings omitted]\n"
        ));
    }
    if eol_files > 0 || eol_lines > 0 {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        let mut parts = Vec::new();
        if eol_files > 0 {
            parts.push(format!(
                "{eol_files} files whose only change is line endings"
            ));
        }
        if eol_lines > 0 {
            parts.push(format!(
                "{eol_lines} lines changed only in line endings, shown as unchanged"
            ));
        }
        output.push_str(&format!("[RTK: diff: {}]\n", parts.join("; ")));
    }
    output
}

static HUNK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@@ -\d+(?:,(\d+))? \+\d+(?:,(\d+))? @@").unwrap());
static FILE_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:index |old mode |new mode |deleted file mode |new file mode |similarity index ",
        r"|dissimilarity index |rename from |rename to |copy from |copy to |--- |\+\+\+ )"
    ))
    .unwrap()
});

/// A diff line's content without its marker and line terminator. Terminal capture has
/// usually stripped the CR already, so a CRLF→LF rewrite shows as `-x` / `+x`.
fn content(line: &str) -> &str {
    line[1..].trim_end_matches(['\n', '\r'])
}

/// Turn `-`/`+` pairs that differ only in line endings into context, and drop files
/// whose whole diff was that. Hunk line counts are unchanged: a pair of one removed and
/// one added line becomes one context line on both sides.
fn line_ending_only_changes(input: &str) -> (String, usize, usize) {
    if !input.starts_with("diff --git ") && !input.contains("\ndiff --git ") {
        return (input.to_string(), 0, 0);
    }
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut output = String::with_capacity(input.len());
    let (mut files, mut total_pairs) = (0, 0);
    let mut i = 0;
    while i < lines.len() {
        if !DIFF_HEADER.is_match(lines[i]) {
            output.push_str(lines[i]);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < lines.len() && FILE_HEADER.is_match(lines[i]) {
            i += 1;
        }
        let header_end = i;
        let mut hunks = String::new();
        let (mut pairs, mut changes, mut any_hunk) = (0, false, false);
        while i < lines.len() {
            let Some(end) = hunk_end(&lines, i) else {
                break;
            };
            any_hunk = true;
            let (body, hunk_pairs, hunk_changes) = pair_line_endings(&lines[i + 1..end]);
            pairs += hunk_pairs;
            if hunk_changes {
                changes = true;
                hunks.push_str(lines[i]);
                hunks.push_str(&body);
            } else if hunk_pairs == 0 {
                // Nothing to rewrite (pure context is unusual); keep it verbatim.
                lines[i..end].iter().for_each(|line| hunks.push_str(line));
            }
            i = end;
        }
        if any_hunk && !changes && pairs > 0 {
            // Keep the file name and any mode/rename lines; drop blob ids and the body.
            files += 1;
            output.push_str(lines[start]);
            for line in &lines[start + 1..header_end] {
                if !(line.starts_with("index ")
                    || line.starts_with("--- ")
                    || line.starts_with("+++ "))
                {
                    output.push_str(line);
                }
            }
        } else {
            total_pairs += pairs;
            lines[start..header_end]
                .iter()
                .for_each(|line| output.push_str(line));
            output.push_str(&hunks);
        }
    }
    (output, files, total_pairs)
}

/// The index just past the hunk starting at `at`, using the header's line counts.
fn hunk_end(lines: &[&str], at: usize) -> Option<usize> {
    let captures = HUNK.captures(lines[at])?;
    let count = |i: usize| {
        captures
            .get(i)
            .map_or(Some(1), |m| m.as_str().parse::<usize>().ok())
    };
    let (mut old, mut new) = (count(1)?, count(2)?);
    let mut i = at + 1;
    while old > 0 || new > 0 {
        let line = lines.get(i)?;
        match line.as_bytes().first()? {
            // An empty context line may have lost its space to trailing-whitespace trimming.
            b' ' | b'\n' | b'\r' => {
                old = old.checked_sub(1)?;
                new = new.checked_sub(1)?;
            }
            b'-' => old = old.checked_sub(1)?,
            b'+' => new = new.checked_sub(1)?,
            b'\\' => {}
            _ => return None,
        }
        i += 1;
    }
    // A trailing "\ No newline at end of file" belongs to the hunk.
    while lines.get(i).is_some_and(|line| line.starts_with('\\')) {
        i += 1;
    }
    Some(i)
}

/// Returns the rewritten body, the pairs turned into context, and whether a real change remains.
fn pair_line_endings(body: &[&str]) -> (String, usize, bool) {
    let mut output = String::new();
    let (mut pairs, mut changes) = (0, false);
    let mut i = 0;
    while i < body.len() {
        if !body[i].starts_with('-') {
            changes |= body[i].starts_with('+');
            output.push_str(body[i]);
            i += 1;
            continue;
        }
        let removed_end = i + body[i..].iter().take_while(|l| l.starts_with('-')).count();
        let added_end = removed_end
            + body[removed_end..]
                .iter()
                .take_while(|l| l.starts_with('+'))
                .count();
        let (removed, added) = (&body[i..removed_end], &body[removed_end..added_end]);
        let same = |a: &&str, b: &&str| content(a) == content(b);
        let prefix = removed
            .iter()
            .zip(added)
            .take_while(|(a, b)| same(a, b))
            .count();
        let limit = removed.len().min(added.len()) - prefix;
        let suffix = removed[prefix..]
            .iter()
            .rev()
            .zip(added[prefix..].iter().rev())
            .take(limit)
            .take_while(|(a, b)| same(a, b))
            .count();
        for line in &added[..prefix] {
            output.push(' ');
            output.push_str(&line[1..]);
        }
        let (middle_removed, middle_added) = (
            &removed[prefix..removed.len() - suffix],
            &added[prefix..added.len() - suffix],
        );
        middle_removed
            .iter()
            .chain(middle_added)
            .for_each(|line| output.push_str(line));
        for line in &added[added.len() - suffix..] {
            output.push(' ');
            output.push_str(&line[1..]);
        }
        pairs += prefix + suffix;
        changes |= !middle_removed.is_empty() || !middle_added.is_empty();
        i = added_end;
    }
    (output, pairs, changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WARN: &str = "warning: in the working copy of 'src/a.rs', LF will be replaced by CRLF the next time Git touches it\n";

    #[test]
    fn line_ending_warnings_become_one_count() {
        let input = format!("{WARN}{WARN} M src/a.rs\n");
        let output = filter(&input);
        assert_eq!(
            output,
            " M src/a.rs\n[RTK: 2 Git line-ending (LF/CRLF) warnings omitted]\n"
        );
    }

    #[test]
    fn index_lines_only_inside_diff_headers() {
        let input = "diff --git a/x b/x\nnew file mode 100644\nindex 0000000..e69de29\n--- /dev/null\n+++ b/x\n@@ -0,0 +1 @@\n+index 1234567..89abcde 100644\n";
        let output = filter(input);
        assert!(!output.contains("\nindex 0000000"), "{output}");
        assert!(output.contains("new file mode 100644"));
        assert!(output.contains("+index 1234567..89abcde 100644"));
    }

    #[test]
    fn line_ending_only_file_collapses_to_its_name() {
        // A CRLF→LF rewrite after terminal capture stripped the CRs.
        let input = "diff --git a/eol.txt b/eol.txt\nindex 1111111..2222222 100644\n--- a/eol.txt\n+++ b/eol.txt\n@@ -1,3 +1,3 @@\n-one\r\n-two\r\n-three\r\n+one\n+two\n+three\ndiff --git a/real.rs b/real.rs\nindex 3333333..4444444 100644\n--- a/real.rs\n+++ b/real.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n same\n";
        let output = filter(input);
        assert_eq!(
            output,
            "diff --git a/eol.txt b/eol.txt\ndiff --git a/real.rs b/real.rs\n--- a/real.rs\n+++ b/real.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n same\n[RTK: diff: 1 files whose only change is line endings]\n"
        );
    }

    #[test]
    fn mixed_hunk_keeps_real_change_and_counts() {
        let input = "diff --git a/m.rs b/m.rs\n--- a/m.rs\n+++ b/m.rs\n@@ -1,4 +1,4 @@\n-a\n-b\n-c\n-d\n+a\n+B\n+c\n+d\n";
        let output = filter(input);
        assert_eq!(
            output,
            "diff --git a/m.rs b/m.rs\n--- a/m.rs\n+++ b/m.rs\n@@ -1,4 +1,4 @@\n a\n-b\n+B\n c\n d\n[RTK: diff: 3 lines changed only in line endings, shown as unchanged]\n"
        );
    }

    #[test]
    fn trimmed_empty_context_lines_stay_inside_the_hunk() {
        // Captured output often loses the single space of an empty context line.
        let input = "diff --git a/t.rs b/t.rs\n--- a/t.rs\n+++ b/t.rs\n@@ -1,3 +1,3 @@\n-x\r\n+x\n\n ctx\n@@ -9 +9 @@\n-y\n+z\n";
        let output = filter(input);
        assert_eq!(
            output,
            "diff --git a/t.rs b/t.rs\n--- a/t.rs\n+++ b/t.rs\n@@ -9 +9 @@\n-y\n+z\n[RTK: diff: 1 lines changed only in line endings, shown as unchanged]\n"
        );
    }

    #[test]
    fn line_ending_hunks_drop_but_other_hunks_stay() {
        let input = "diff --git a/h.rs b/h.rs\n--- a/h.rs\n+++ b/h.rs\n@@ -1,1 +1,1 @@\n-x\r\n+x\n@@ -9 +9 @@\n-y\n+z\n\\ No newline at end of file\ntrailing command output\n";
        let output = filter(input);
        assert_eq!(
            output,
            "diff --git a/h.rs b/h.rs\n--- a/h.rs\n+++ b/h.rs\n@@ -9 +9 @@\n-y\n+z\n\\ No newline at end of file\ntrailing command output\n[RTK: diff: 1 lines changed only in line endings, shown as unchanged]\n"
        );
    }

    #[test]
    fn status_help_only_in_human_status() {
        let status = "On branch main\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n\tmodified:   a.rs\n\nno changes added to commit (use \"git add\" and/or \"git commit -a\")\n";
        let output = filter(status);
        assert!(!output.contains("(use \"git add <file>"), "{output}");
        assert!(output.contains("modified:   a.rs"));
        assert!(output.contains("no changes added to commit"));
        let other = "  (use \"git add <file>...\" to update what will be committed)\n";
        assert_eq!(filter(other), other);
    }
}
