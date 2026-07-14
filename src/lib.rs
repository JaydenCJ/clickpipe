//! clickpipe — a pipe filter that turns file paths, URLs, and issue IDs in
//! any command's output into clickable OSC 8 terminal hyperlinks.
//!
//! The library crate exposes the detection and rendering layers so they can
//! be unit-tested and reused; the `clickpipe` binary wires them into a
//! stdin-to-stdout filter (see `src/main.rs`).
//!
//! Pipeline per line: [`ansi`] splits the raw line into escape sequences and
//! plain text, [`scan`] runs the [`urls`], [`paths`] and [`issues`] detectors
//! over the plain text and resolves overlaps, and [`ansi::AnsiLine::render`]
//! stitches the line back together with [`osc8`] hyperlink wrappers — leaving
//! every byte the upstream tool printed intact.

pub mod ansi;
pub mod cli;
pub mod giturl;
pub mod issues;
pub mod osc8;
pub mod paths;
pub mod scan;
pub mod uri;
pub mod urls;
