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
static NEXT_STAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Creating an optimized production build|Running TypeScript|Collecting page data using \d+ workers|Generating static pages using \d+ workers \(\d+/\d+\)|Finalizing page optimization) \.\.\.$").unwrap()
});
static VITE_BANNER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\[info\] |ℹ )?vite v\d+\.[^\r\n]+ building [^\r\n]*production\.\.\.$")
        .unwrap()
});
static NUXT_BANNER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[● ]*Nuxt \d+\.").unwrap());
static NUXT_ASSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:\[info\] )?(?:[├└]─ )?(\.nuxt/dist/client/|\.output/server/)([^\r\n]+)$")
        .unwrap()
});

pub fn recognizes(input: &str) -> bool {
    NODE_TOTALS.is_match(input)
        || VITE_BANNER.is_match(input)
        || NUXT_BANNER.is_match(input)
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
    let mut filtered = String::with_capacity(output.len());
    let mut next = false;
    let mut vite = false;
    for line in output.split_inclusive('\n') {
        let text = line.trim();
        let message = text
            .strip_prefix("[info] ")
            .or_else(|| text.strip_prefix("ℹ "))
            .unwrap_or(text);
        if text.starts_with("> ") || text.starts_with("$ ") {
            next = false;
            vite = false;
        }
        if NEXT_BANNER.is_match(text) {
            next = true;
            vite = false;
        }
        if VITE_BANNER.is_match(text) {
            vite = true;
            next = false;
        }
        if (next && NEXT_STAGE.is_match(text))
            || (vite
                && matches!(
                    message,
                    "transforming..." | "rendering chunks..." | "computing gzip size..."
                ))
        {
            continue;
        }
        filtered.push_str(line);
        if text.starts_with("Route (") || text.starts_with("> Build error occurred") {
            next = false;
        }
        if message.starts_with("✓ built in ")
            || message.starts_with("✗ Build failed in ")
            || message == "error during build:"
        {
            vite = false;
        }
    }
    if !NUXT_BANNER.is_match(input) {
        return filtered;
    }
    // Preserve every artifact and size; factor repeated directory prefixes out
    // of contiguous Nuxt tables. Never truncate the list or merge build tables.
    let mut compact = String::with_capacity(filtered.len());
    let mut lines = filtered.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let Some(asset) = NUXT_ASSET.captures(line.trim()) else {
            compact.push_str(line);
            continue;
        };
        let root = asset.get(1).unwrap().as_str();
        let mut entries = vec![asset.get(2).unwrap().as_str()];
        while let Some(next) = lines.peek() {
            let Some(asset) = NUXT_ASSET.captures(next.trim()) else {
                break;
            };
            if &asset[1] != root {
                break;
            }
            entries.push(asset.get(2).unwrap().as_str());
            lines.next();
        }
        if entries.len() == 1 {
            compact.push_str(line);
        } else {
            compact.push_str(&format!("{root}:\n"));
            for entry in entries {
                compact.push_str("  ");
                compact.push_str(entry);
                compact.push('\n');
            }
        }
    }
    compact
}
