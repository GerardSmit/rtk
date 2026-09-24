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

/// `file(line,col): error TS1234: message`, as `tsc`, `vue-tsc` and `nuxt typecheck` print it.
static TS_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\S[^\r\n]*?)\((\d+,\d+)\): error (TS\d+): ([^\r\n]*)$").unwrap()
});

pub fn recognizes(input: &str) -> bool {
    input
        .lines()
        .filter(|line| TS_ERROR.is_match(line.trim()))
        .nth(2)
        .is_some()
        || NODE_TOTALS.is_match(input)
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
    group_typescript(&minimal_build_output(&filtered))
}

/// One TypeScript error: file, `line,col`, code, and the message with any indented
/// elaboration lines that followed it.
struct TsError<'a> {
    file: &'a str,
    position: &'a str,
    code: &'a str,
    message: String,
}

/// Three or more TypeScript errors become one block where the first one was. Every
/// file, position, code and message stays; what goes is repetition. The block is laid
/// out by file then message, or by message then file, whichever is shorter: a few
/// messages across many files read best message-first, many messages per file file-first.
fn group_typescript(input: &str) -> String {
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut errors: Vec<TsError> = Vec::new();
    let mut output = String::with_capacity(input.len());
    let mut first = None;
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let Some(c) = TS_ERROR.captures(line.trim()) else {
            output.push_str(line);
            continue;
        };
        first.get_or_insert(output.len());
        let (_, [file, position, code, message]) = c.extract();
        let mut message = message.to_string();
        // tsc indents the elaboration of an error ("Type 'A' is not assignable ...").
        while index < lines.len()
            && lines[index].starts_with([' ', '\t'])
            && !lines[index].trim().is_empty()
            && !TS_ERROR.is_match(lines[index].trim())
        {
            message.push_str("\n    ");
            message.push_str(lines[index].trim());
            index += 1;
        }
        errors.push(TsError {
            file,
            position,
            code,
            message,
        });
    }
    let Some(first) = first.filter(|_| errors.len() >= 3) else {
        return input.to_string();
    };
    let by_file = typescript_by_file(&errors);
    let by_message = typescript_by_message(&errors);
    let (layout, body) = if by_file.len() <= by_message.len() {
        ("file, then message", by_file)
    } else {
        ("message, then file", by_message)
    };
    let mut block = format!(
        "tsc: {} errors grouped by {layout} (every location kept):\n{body}",
        errors.len()
    );
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

/// Keys in first-seen order, each with its members.
fn ordered_groups<'e, 'a: 'e, K: PartialEq>(
    errors: impl IntoIterator<Item = &'e TsError<'a>>,
    key: impl Fn(&'e TsError<'a>) -> K,
) -> Vec<(K, Vec<&'e TsError<'a>>)> {
    let mut groups: Vec<(K, Vec<&TsError>)> = Vec::new();
    for error in errors {
        let k = key(error);
        match groups.iter_mut().find(|(seen, _)| *seen == k) {
            Some((_, members)) => members.push(error),
            None => groups.push((k, vec![error])),
        }
    }
    groups
}

fn typescript_by_file(errors: &[TsError]) -> String {
    let mut body = String::new();
    for (file, members) in ordered_groups(errors, |e| e.file) {
        if let [one] = members.as_slice() {
            body.push_str(&format!(
                "{file}({}): {}: {}\n",
                one.position, one.code, one.message
            ));
            continue;
        }
        body.push_str(&format!("{file}:\n"));
        for ((code, message), same) in
            ordered_groups(members.iter().copied(), |e| (e.code, e.message.as_str()))
        {
            let positions: Vec<&str> = same.iter().map(|e| e.position).collect();
            body.push_str(&format!("  ({}) {code}: {message}\n", positions.join("; ")));
        }
    }
    body
}

fn typescript_by_message(errors: &[TsError]) -> String {
    let mut body = String::new();
    for ((code, message), members) in ordered_groups(errors, |e| (e.code, e.message.as_str())) {
        if let [one] = members.as_slice() {
            body.push_str(&format!(
                "{}({}): {code}: {message}\n",
                one.file, one.position
            ));
            continue;
        }
        let places: Vec<String> = ordered_groups(members.iter().copied(), |e| e.file)
            .into_iter()
            .map(|(file, at)| {
                let positions: Vec<&str> = at.iter().map(|e| e.position).collect();
                format!("{file}({})", positions.join("; "))
            })
            .collect();
        body.push_str(&format!(
            "{code} ×{}: {message}\n  {}\n",
            members.len(),
            places.join("; ")
        ));
    }
    body
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_errors_per_file_group_file_first() {
        let input = concat!(
            "src/theme.ts(2,10): error TS2724: '\"./api/models\"' has no exported member named 'ItemModel'.\n",
            "src/views/Text.tsx(5,34): error TS2339: Property 'defaultProps' does not exist on type 'Box'.\n",
            "src/views/Text.tsx(22,70): error TS2339: Property 'defaultProps' does not exist on type 'Box'.\n",
            "src/views/Text.tsx(36,35): error TS2339: Property 'defaultProps' does not exist on type 'Box'.\n",
            "src/views/Text.tsx(40,1): error TS2307: Cannot find module 'ui-kit' or its corresponding type declarations.\n",
            "Found 5 errors in 2 files.\n",
        );
        assert_eq!(
            filter(input),
            concat!(
                "tsc: 5 errors grouped by file, then message (every location kept):\n",
                "src/theme.ts(2,10): TS2724: '\"./api/models\"' has no exported member named 'ItemModel'.\n",
                "src/views/Text.tsx:\n",
                "  (5,34; 22,70; 36,35) TS2339: Property 'defaultProps' does not exist on type 'Box'.\n",
                "  (40,1) TS2307: Cannot find module 'ui-kit' or its corresponding type declarations.\n",
                "Found 5 errors in 2 files.\n",
            )
        );
    }

    #[test]
    fn one_error_across_many_files_groups_message_first() {
        let line = |file: &str, pos: &str| {
            format!(
                "src/{file}.ts({pos}): error TS7006: Parameter 'event' implicitly has an 'any' type.\n"
            )
        };
        let input = format!(
            "> tsc --noEmit\n{}{}{}{}",
            line("a", "1,2"),
            line("b", "3,4"),
            line("b", "9,9"),
            line("c", "5,6")
        );
        assert_eq!(
            filter(&input),
            concat!(
                "> tsc --noEmit\n",
                "tsc: 4 errors grouped by message, then file (every location kept):\n",
                "TS7006 ×4: Parameter 'event' implicitly has an 'any' type.\n",
                "  src/a.ts(1,2); src/b.ts(3,4; 9,9); src/c.ts(5,6)\n",
            )
        );
    }

    #[test]
    fn elaboration_lines_stay_with_their_error() {
        let input = concat!(
            "a.ts(1,1): error TS2322: Type 'A' is not assignable to type 'B'.\n",
            "  Types of property 'x' are incompatible.\n",
            "a.ts(2,1): error TS2322: Type 'A' is not assignable to type 'B'.\n",
            "  Types of property 'y' are incompatible.\n",
            "b.ts(3,1): error TS2304: Cannot find name 'z'.\n",
        );
        let output = filter(input);
        assert!(
            output.contains("Types of property 'x' are incompatible."),
            "{output}"
        );
        assert!(
            output.contains("Types of property 'y' are incompatible."),
            "{output}"
        );
        assert!(output.contains("(1,1)") && output.contains("(2,1)") && output.contains("(3,1)"));
    }

    #[test]
    fn two_errors_stay_verbatim() {
        let input = "a.ts(1,1): error TS2304: Cannot find name 'z'.\nb.ts(3,1): error TS2304: Cannot find name 'z'.\n";
        assert_eq!(filter(input), input);
    }
}
