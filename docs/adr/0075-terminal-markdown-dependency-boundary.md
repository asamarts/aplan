# ADR-0075: Terminal Markdown dependency boundary

- Status: Accepted
- Date: 2026-11-24
- Task: `A2-113`
- References: `docs/design/terminal-markdown-evaluation.md`,
  `docs/design/workflow-ergonomics.md#2-terminal-markdown-rendering`, `docs/design/tui.md`

## Context

`aplan` needs to render Markdown-authored task bodies in human CLI output (`aplan show --body`)
and in the interactive TUI (`aplan-tui`), while keeping artifact outputs, JSON payloads, and
explicit raw-body routes byte-exact. Writing and maintaining a CommonMark/GFM parser by hand would
turn Markdown edge cases into a permanent parser and security burden, whereas adopting a full
terminal Markdown renderer risks pulling a second terminal stack, conflicting theme/capability
policy, copyleft licenses, or megabytes of syntax-highlighting data.

Isolated comparative prototypes (`docs/design/terminal-markdown-evaluation.md`) measured
`pulldown-cmark` (`0.13.6`), `termimad` (`0.34.1`), `pulldown-cmark-mdcat` (`2.20.2`),
parser-only `comrak` (`0.39.1`), Unicode wrapping crates (`unicode-width` `0.2.3`,
`unicode-segmentation` `1.13.3`, `unicode-linebreak` `0.1.5`), and `syntect` (`5.3.0`) against
`aplan`'s Rust `1.88` MSRV floor, headless CLI build boundary, and supply-chain gates.

## Decision

1. **Select `pulldown-cmark` as the Markdown parser dependency in `aplan-output`**:
   - Depend on `pulldown-cmark` with `default-features = false` (major/minor series compatible with
     the workspace MSRV `1.88`, measured at `0.13.6`, without freezing a patch version as
     architecture).
   - Enable only the required GFM parser options (`ENABLE_TABLES`, `ENABLE_STRIKETHROUGH`,
     `ENABLE_TASKLISTS`). Keep `ENABLE_SMART_PUNCTUATION` disabled so rendering never alters
     author characters.
   - Keep all `pulldown-cmark` types private to `aplan-output`. Expose only an `aplan`-owned
     transport-neutral semantic document consumed by both the `anstyle` CLI sink and `aplan-tui`'s
     Ratatui sink.

2. **Reuse the workspace's existing Unicode wrapping stack**:
   - Use `aplan-output` and `aplan-tui`'s already-declared `unicode-width` (`0.2`) and
     `unicode-segmentation` (`1.13`) dependencies for grapheme-safe cell width and wrapping; never
     claim grapheme-safe wrapping from `unicode-width` alone.
   - `unicode-linebreak` (`0.1`, `Apache-2.0`, MSRV `1.56`) is pre-approved as a single optional
     table crate if UAX #14 line-breaking opportunities are needed beyond whitespace and
     extended-grapheme folding.

3. **Decline `syntect` for the baseline renderer**:
   - Ship baseline fenced and inline code with semantic code styling and a visible language label.
   - Revisit token-level syntax highlighting only behind an explicit future measurement that meets
     dependency, license, MSRV, and binary-size budgets.

4. **Enforce strict sanitization and non-I/O boundaries**:
   - All text, code, link/image metadata, and raw HTML slices pass through `aplan`'s existing
     control-character and ANSI/CSI/OSC sanitizer before styling is applied.
   - Markdown rendering performs no terminal queries, OSC-8 hyperlinks, image protocols, file
     reads, subprocess execution, or network requests, and adds no terminal backend crate to
     `aplan-output`.

## Alternatives considered and rejected

- **`termimad` (measured `0.34.1`)**: Rejected. Pulls 64 crates including `crossterm` into
  `aplan-output` (violating the headless CLI boundary), duplicates `unicode-width 0.1` alongside
  workspace `0.2`, uses a non-CommonMark parser (`minimad`), and owns its own ANSI/skin policy.
- **`pulldown-cmark-mdcat` library (measured `2.20.2`)**: Rejected. Fails `aplan`'s Rust `1.88`
  MSRV (`1.95`), pulls 324 crates even with `default-features = false`, introduces copyleft /
  weak-copyleft licenses (`MPL-2.0`, `LPPL-1.3c`, `LGPL-2.1-or-later`), pulls `syntect`
  unconditionally, and owns terminal capability/image/OSC policy.
- **`comrak` parser-only (measured `0.39.1`)**: Rejected as default; retained as documented
  fallback. Respects MSRV (`1.65`) and BSD-2-Clause licensing, but requires 21 crates (vs. 4),
  larger stripped binary delta (`+542 KiB` vs. `+391 KiB`), higher peak RSS (`24.1 MiB` vs.
  `14.0 MiB` due to arena AST allocation), and 3.7x lower throughput for a display-only pass.
- **`syntect` (measured `5.3.0`, `default-fancy`)**: Declined for baseline. Adds 49 crates,
  undeclared MSRV, and `+2.54 MiB` stripped binary size (over 11x the entire TUI feature's
  recorded binary cost).

## Consequences

- `aplan-output` gains one minimal, pure-Rust, MIT-licensed CommonMark/GFM pull parser (`+4`
  resolved crates, `+391 KiB` stripped delta, MSRV `1.71.1`) while keeping `--no-default-features`
  headless builds free of `crossterm`/`ratatui`.
- Downstream tasks (`A2-114` semantic document and CLI/TUI sinks, `A2-115` CLI/TUI surface wiring)
  build on one shared semantic AST and executable member census without leaking parser types or
  duplicating terminal layout logic.
