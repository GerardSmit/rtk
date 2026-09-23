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
    output
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
