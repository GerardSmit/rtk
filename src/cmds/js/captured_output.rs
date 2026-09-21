//! Surgical filtering of completed JavaScript command output, including shell batches.
use regex::Regex;
use std::sync::LazyLock;

static NODE_TOTALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?m)^ℹ tests \d+\r?\nℹ suites \d+\r?\nℹ pass (\d+)\r?\n",
        r"ℹ fail \d+\r?\nℹ cancelled \d+\r?\nℹ skipped \d+\r?\n",
        r"ℹ todo \d+\r?\nℹ duration_ms [\d.]+\r?$"
    ))
    .unwrap()
});
static NODE_PASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*✔ [^\r\n]+ \([\d.]+ms\)\r?\n").unwrap());
static NEXT_BANNER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[ \t]*▲ Next\.js \d+\.").unwrap());
static NEXT_PROGRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?m)^(?:[ \t]*(?:Running TypeScript|Collecting page data using \d+ workers|",
        r"Finalizing page optimization)[ \t]+\.{1,3}[ \t]*|",
        r"[ \t]*Generating static pages using \d+ workers \(\d+/\d+\)[ \t]+\[[ =]*\][ \t]*)+",
        r"(✓[^\r\n]*)"
    ))
    .unwrap()
});

pub fn recognizes(input: &str) -> bool {
    NODE_TOTALS.is_match(input)
        || (NEXT_BANNER.is_match(input) && input.contains("Creating an optimized production build"))
}

pub fn filter(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut end = 0;
    // Keep all totals and failure details verbatim. Only complete passing rows
    // before a complete reporter summary can be removed.
    for totals in NODE_TOTALS.captures_iter(input) {
        let summary = totals.get(0).unwrap();
        let segment = &input[end..summary.start()];
        let count = NODE_PASS.find_iter(segment).count();
        let passed = totals[1].parse::<usize>().unwrap_or(0);
        if count > 0 && count <= passed {
            output.push_str(&NODE_PASS.replace_all(segment, ""));
            output.push_str(&format!("[RTK: {count} passing test rows omitted]\n"));
        } else {
            output.push_str(segment);
        }
        output.push_str(summary.as_str());
        end = summary.end();
    }
    output.push_str(&input[end..]);
    if NEXT_BANNER.is_match(input) && input.contains("Creating an optimized production build") {
        // Delete only repeated animation frames immediately followed by a
        // completion line. Unknown lines, warnings and route tables stay intact.
        output = NEXT_PROGRESS.replace_all(&output, "$1").into_owned();
    }
    output
}
