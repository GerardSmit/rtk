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
    Regex::new(r"^(?:\[info\] )?(?:[├└]─ )?(?:(?:node_modules/\.cache/nuxt/)?\.nuxt/dist/client/|\.output/server/)[^\r\n]+(?:kB| B)(?:[^\r\n]*)$")
        .unwrap()
});
static DEPRECATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\(node:\d+\) \[DEP\d+\] DeprecationWarning: ").unwrap());
static VITE_ASSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\S+/)\S+\s+\d+(?:\.\d+)? kB(?:\s+│ gzip:.*)?$").unwrap());

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
    minimal_build_output(&filtered)
}

fn minimal_build_output(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut framework = "";
    let mut deprecation = false;
    let mut vite_directory = String::new();
    let mut routes = false;
    let mut nuxt_server_output = false;
    for line in input.split_inclusive('\n') {
        let text = line.trim();
        if DEPRECATION.is_match(text) {
            deprecation = true;
            continue;
        }
        if deprecation && text.starts_with("(Use `node --trace-deprecation") {
            deprecation = false;
            continue;
        }
        deprecation = false;
        if text.starts_with("> ") || text.starts_with("$ ") {
            framework = "";
            vite_directory.clear();
            routes = false;
        }
        if text == "┌  Building Nuxt for production..." || NUXT_BANNER.is_match(text) {
            framework = "nuxt";
            nuxt_server_output = false;
            continue;
        }
        if NEXT_BANNER.is_match(text) {
            framework = "next";
            continue;
        }
        if VITE_BANNER.is_match(text) {
            if framework != "nuxt" {
                framework = "vite";
            }
            continue;
        }
        if !framework.is_empty() && text.is_empty() {
            continue;
        }
        match framework {
            "nuxt" => {
                if NUXT_ASSET.is_match(text) && text.contains(".output/server/") {
                    nuxt_server_output = true;
                }
                if NUXT_ASSET.is_match(text)
                    || matches!(
                        text,
                        "│" | "[log] │" | "[info] Building client..." | "[info] Building server..."
                    )
                    || text.starts_with("●  Nitro preset:")
                    || text.starts_with("✓ ") && text.ends_with(" modules transformed.")
                    || text.starts_with("[info] ✓ built in ")
                    || text.starts_with("[success] Client built in ")
                    || text.starts_with("[success] Server built in ")
                    || text.starts_with("[info] [nitro] Building Nuxt Nitro server (")
                    || text.starts_with("[success] [nitro] Generated public ")
                {
                    continue;
                }
                if text == "[success] [nitro] Nuxt Nitro server built" {
                    continue;
                }
                if let Some(total) = text.strip_prefix("Σ Total size: ") {
                    if nuxt_server_output {
                        output.push_str(".output/server generated. ");
                    }
                    output.push_str(&format!("Total size: {total}\n"));
                    continue;
                }
                if let Some(preview) =
                    text.strip_prefix("[success] [nitro] You can preview this build using ")
                {
                    output.push_str(&format!("Preview: {preview}\n"));
                    continue;
                }
                if text == "└  ✨ Build complete!" {
                    output.push_str("Nuxt build succeeded.\n");
                    framework = "";
                    continue;
                }
            }
            "vite" => {
                if let Some(asset) = VITE_ASSET.captures(text) {
                    // Default Vite asset tables use dist/ and dist/assets/. Keep
                    // custom directories verbatim rather than guessing their root.
                    let directory = asset[1].strip_suffix("assets/").unwrap_or(&asset[1]);
                    if directory != vite_directory {
                        output.push_str(&format!("{directory} generated.\n"));
                        vite_directory = directory.to_string();
                    }
                    continue;
                }
                if text.starts_with("✓ ") && text.ends_with(" modules transformed.") {
                    continue;
                }
                if let Some(time) = text.strip_prefix("✓ built in ") {
                    output.push_str(&format!("Vite build succeeded ({time}).\n"));
                    framework = "";
                    continue;
                }
            }
            "next" => {
                if text.starts_with("Route (") {
                    routes = true;
                    output.push_str("Next.js build succeeded; routes generated.\n");
                    continue;
                }
                if text.starts_with("✓ Running next.config")
                    || text.starts_with("✓ Compiled successfully in ")
                    || text.starts_with("Finished TypeScript in ")
                    || text.starts_with("✓ Finished TypeScript in ")
                    || text.starts_with("✓ Generating static pages using ")
                    || text.starts_with("✓ Collecting page data using ")
                    || text.starts_with("✓ Finalizing page optimization in ")
                    || routes
                        && (text.starts_with("┌ ")
                            || text.starts_with("├ ")
                            || text.starts_with("└ ")
                            || text.starts_with("○  (Static)")
                            || text.starts_with("ƒ  (Dynamic)"))
                {
                    continue;
                }
            }
            _ => {}
        }
        output.push_str(line);
    }
    output
}
