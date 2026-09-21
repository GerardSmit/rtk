//! RTK's output filters, callable without spawning RTK or enabling its tracking.
//!
//! The caller owns command execution and retains the original output. Select a
//! filter only when its input format matches: `git-status` expects porcelain,
//! `go-test` expects JSON events, and `ruff-check` expects JSON diagnostics.
//!
//! ```
//! use std::borrow::Cow;
//! let raw = "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
//! let compact = rtk_filter::filter("cargo-test", raw).unwrap_or(Cow::Borrowed(raw));
//! assert!(compact.contains("5"));
//! ```

// Shared CLI types have methods that the filtering-only build does not call.
#![allow(dead_code)]

pub use core::utils::{join_with_overflow, strip_ansi, truncate};
use std::borrow::Cow;

/// Larger inputs bypass filtering without allocation. This bounds input size,
/// not all temporary allocations inside individual filters.
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;

/// Filter captured text in memory. An unknown name returns `None`.
///
/// Oversized input, empty output, output that is not smaller, and an unwinding
/// filter panic return the original borrowed text. This does not detect a wrong
/// input format, guarantee lossless compression, or provide a timeout. Retain raw
/// output for recovery. Panic recovery requires the consumer's unwind profile.
pub fn filter<'a>(name: &str, input: &'a str) -> Option<Cow<'a, str>> {
    let filter = cmds::system::pipe_cmd::resolve_filter(name)?;
    if input.len() > MAX_INPUT_BYTES || input.is_empty() {
        return Some(Cow::Borrowed(input));
    }
    let output = cmds::system::pipe_cmd::apply_filter(filter, input);
    Some(match output {
        output if !output.trim().is_empty() && output.len() < input.len() => Cow::Owned(output),
        _ => Cow::Borrowed(input),
    })
}

/// Whether a filter name is supported, including RTK's aliases.
pub fn supports_filter(name: &str) -> bool {
    cmds::system::pipe_cmd::resolve_filter(name).is_some()
}

mod parser;

mod core {
    pub mod guard;
    pub mod stream;
    pub mod truncate;
    pub mod utils;

    // Embedded filtering never loads a user's CLI configuration.
    pub mod config {
        pub struct Limits {
            pub passthrough_max_chars: usize,
        }
        pub fn limits() -> Limits {
            Limits {
                passthrough_max_chars: 2000,
            }
        }
    }

    // Recovery belongs to the embedding host, not RTK's filesystem/SQLite stores.
    pub mod tee {
        pub fn force_tee_hint(_: &str, _: &str) -> Option<String> {
            None
        }
        pub fn force_tee_tail_hint(_: &str, _: &str, _: usize) -> Option<String> {
            None
        }
    }
}

mod cmds {
    pub mod git {
        pub mod git_cmd;
    }
    pub mod go {
        pub mod go_cmd;
    }
    pub mod js {
        pub mod captured_output;
        pub mod prettier_cmd;
        pub mod tsc_cmd;
        pub mod vitest_cmd;
    }
    pub mod php {
        pub mod ecs_cmd;
        pub mod phpstan_cmd;
        pub mod phpunit_cmd;
        pub mod pint_cmd;
        pub mod test_output;
        pub mod utils;
    }
    pub mod python {
        pub mod mypy_cmd;
        pub mod pytest_cmd;
        pub mod ruff_cmd;
        pub mod sqlfluff_cmd;
    }
    pub mod rust {
        pub mod cargo_cmd;
    }
    pub mod system {
        pub mod captured_output;
        pub mod ctest_cmd;
        pub mod log_cmd;
        pub mod pipe_cmd;
    }
}
