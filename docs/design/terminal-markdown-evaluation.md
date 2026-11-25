# Terminal Markdown dependency evaluation

Status: measurement complete (`A2-113`, `ADR-0075`). Confirms the provisional recommendation in
`docs/design/workflow-ergonomics.md#2-terminal-markdown-rendering`: select `pulldown-cmark` with
default features disabled, reuse `aplan-output` and `aplan-tui`'s existing `unicode-width` and
`unicode-segmentation` dependencies, and ship baseline semantic code styling without `syntect`.

## Measurement environment and corpus

- **Host**: Linux x86_64, Rust 1.99.1 (`edition = "2024"`), release profile with `strip = true`.
- **Baseline**: empty Rust binary reading the identical embedded corpus (`371,832 B` stripped,
  `0.14 s` clean release build, `13,796 KiB` peak RSS).
- **Existing workspace terminal/Unicode baseline** (from `Cargo.toml` and `docs/design/tui.md`):
  - Workspace MSRV floor: **Rust 1.88**.
  - Existing direct dependencies in both `aplan-output` and `aplan-tui`: `unicode-width` `0.2.3`
    (MSRV `1.66`, `MIT OR Apache-2.0`) and `unicode-segmentation` `1.13.3` (MSRV `1.85.0`,
    `MIT OR Apache-2.0`).
  - Existing TUI terminal stack (`aplan-tui` behind default-on `tui` feature): `ratatui` `0.30.2`
    (`default-features = false, features = ["crossterm"]`) and `crossterm` `0.29.0`
    (`use-dev-tty`), previously measured at `+229,456 B` (`+1.79%`) stripped release binary delta
    while keeping `--no-default-features` headless CLI builds free of terminal backends.
- **Shared corpus** (`804 B`, repeated 2,000 times per parse/render run; 500 times for `syntect`):
  headings 1-3, emphasis, strong emphasis, strikethrough, inline code, fenced Rust code block,
  nested blockquotes, ordered/unordered/task lists, thematic break, link, image, GFM table,
  hard break, raw HTML (`<script>`), adversarial CSI/OSC escape sequences, combining marks
  (`e` + combining acute accent U+0301), CJK ideographs, and ZWJ emoji sequences.

## Comparative measurements

### Parser and terminal renderer candidates

| Candidate (measured version) | Crates | Duplicate terminal / Unicode crates | MSRV | License(s) | Clean release build | Stripped binary delta | Throughput (docs/s) | Peak RSS (KiB) |
|---|---:|---|---|---|---|---:|---:|---:|
| Baseline (no parser) | 0 | none | 1.88 | - | 0.14 s | 0 B | - | 13,796 |
| `pulldown-cmark` `0.13.6` (`default-features = false`) | 4 | none (`bitflags`, `memchr` already in lockfile) | 1.71.1 | `MIT` (deps: `MIT`, `MIT OR Apache-2.0`, `Unlicense OR MIT`) | 2.26 s | +400,208 B (+391 KiB) | 144,258 | 13,988 |
| `pulldown-cmark` `0.13.6` + `unicode-width` `0.2.3` + `unicode-segmentation` `1.13.3` + `unicode-linebreak` `0.1.5` | 7 (only `unicode-linebreak` is new beyond `pulldown-cmark` and existing workspace crates) | none | 1.85.0 | `MIT`, `MIT OR Apache-2.0`, `Apache-2.0`, `Unlicense OR MIT` | 2.25 s | +470,776 B (+460 KiB) | 41,247 | 13,988 |
| `termimad` `0.34.1` | 64 | pulls `crossterm 0.29.0` into `aplan-output` and duplicates `unicode-width 0.1.14` alongside workspace `0.2.3` | 1.56 | `MIT` (deps: `MIT`, `Apache-2.0`, `Unicode-3.0`) | 6.50 s | +163,024 B (+159 KiB) | 49,776 | 15,332 |
| `pulldown-cmark-mdcat` `2.20.2` (`default-features = false`) | 324 | pulls `pulldown-cmark 0.10.3` + `0.12.2`, `syntect 5.3.0`, `terminal_size`, `term`, `anstyle` | **1.95** (exceeds 1.88 floor) | **`MPL-2.0 AND Apache-2.0`** (deps include `LPPL-1.3c`, `LGPL-2.1-or-later`) | 54.87 s | not linked (disqualified) | - | - |
| `comrak` `0.39.1` (`default-features = false`) | 21 | none | 1.65 | `BSD-2-Clause` (deps: `MIT`, `Apache-2.0`, `BSD-3-Clause`, `Zlib`) | 2.95 s | +554,840 B (+542 KiB) | 39,196 | 24,120 |

### Syntax highlighting candidate (measured separately)

| Candidate (measured version) | Crates | MSRV | License | Clean build | Stripped binary delta | Throughput (docs/s) | Peak RSS (KiB) |
|---|---:|---|---|---:|---:|---:|---:|
| Baseline semantic code styling + language label | 0 | 1.88 | workspace | 0.00 s | 0 B | - | - |
| `syntect` `5.3.0` (`default-features = false, features = ["default-fancy"]`) | 49 (`fancy-regex`, `regex-automata`, `bincode`, `flate2`, grammar/theme dumps) | undeclared | `MIT` | 7.72 s | **+2,667,352 B (+2.54 MiB)** | 5,358 | 12,384 |

## Integration fit, sanitization, and Unicode wrapping

1. **Terminal and Ratatui integration fit**:
   - `pulldown-cmark` exposes a pure-Rust pull iterator (`Parser::new_ext`) with zero terminal,
     ANSI, I/O, or event-loop dependencies. `aplan-output` can lower its events into a small owned
     semantic document (`Block` / `Span` with exhaustive roles) consumed by both the `anstyle` CLI
     sink and `aplan-tui`'s Ratatui `Line`/`Span` sink without leaking parser types or introducing
     a second terminal stack into headless `--no-default-features` builds.
   - `termimad` couples its non-CommonMark `minimad` parser directly to `crossterm` ANSI output and
     its own skin/palette model, conflicting with `aplan`'s `anstyle` CLI roles, Ratatui TUI
     themes, and headless build contract.
   - `pulldown-cmark-mdcat` is designed as a terminal-writing renderer with its own capability
     probing, OSC-8 hyperlink/image/math handlers, and mandatory `syntect` dependency; it also
     breaks `aplan`'s Rust 1.88 MSRV (`1.95`) and expands the graph by 324 crates including
     copyleft/weak-copyleft licenses (`MPL-2.0`, `LPPL-1.3c`, `LGPL-2.1-or-later`).
   - `comrak` (parser-only) builds cleanly and respects MSRV, but allocates a full arena AST
     (24.1 MiB peak RSS vs 14.0 MiB for `pulldown-cmark`), pulls 21 crates vs 4, adds 542 KiB vs
     391 KiB, and runs 3.7x slower for a display-only sink that needs only a single linear pass.

2. **Sanitization and unsupported syntax**:
   - With `pulldown-cmark`, raw HTML (`Event::Html` / `Event::InlineHtml`) and text (`Event::Text`
     / `Event::Code`) are delivered verbatim as string slices before any styling occurs. `aplan`
     routes every slice through its existing control-character and ANSI/CSI/OSC sanitizer before
     constructing semantic spans.
   - Smart punctuation (`Options::ENABLE_SMART_PUNCTUATION`) remains disabled so pretty-printing
     never mutates author characters.
   - Unsupported extensions (footnotes, math, definition lists, raw HTML tags) render their
     sanitized visible text rather than vanishing silently or invoking external resources.

3. **Unicode grapheme segmentation, display width, and line breaking**:
   - `unicode-width` alone does not define grapheme cluster boundaries (splitting between a base
     character and combining accent or inside a ZWJ emoji corrupts terminal cells).
   - Both `unicode-width` (`0.2.3`) and `unicode-segmentation` (`1.13.3`) are already direct
     workspace dependencies of `aplan-output` and `aplan-tui`, adding zero new packages or
     duplicate versions.
   - `unicode-linebreak` (`0.1.5`, `Apache-2.0`, MSRV `1.56`, `+70.6 KiB` stripped delta) is
     measured and acceptable as a single small table crate if UAX #14 line-breaking opportunities
     are needed beyond whitespace + extended-grapheme folding.

4. **Syntax highlighting (`syntect`)**:
   - Adding `syntect` even with `default-features = false, features = ["default-fancy"]` adds 49
     crates and `+2.54 MiB` to the stripped release binary -- more than 11x the entire TUI feature's
     recorded binary cost (`229 KiB`).
   - Baseline terminal Markdown rendering therefore ships with semantic code styling plus a visible
     language label, with zero highlighting grammar bundle.

## Maintenance and advisory posture

- `pulldown-cmark` is actively maintained in the `pulldown-cmark` organization, pure Rust with
  minimal transitive surface (`bitflags`, `memchr`, `pulldown-cmark-escape`), MIT-licensed, and
  well-established across the Rust documentation ecosystem (including `rustdoc`).
- Any admitted dependency remains subject to `aplan`'s workspace `cargo deny` / supply-chain
  advisory, license, and duplicate-version checks in CI.

## Conclusion

The measurements confirm the provisional recommendation in `docs/design/workflow-ergonomics.md`:
no change to the direction is required. See `docs/adr/0075-terminal-markdown-dependency-boundary.md`
for the binding dependency and compatibility policy.
