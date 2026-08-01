//! MCP server for aplan.
//!
//! An MCP server is table stakes in this category (beads, Backlog.md,
//! task-master and mdplanner all expose one; HLD section 11), so shipping one is
//! not differentiation. What it does with `close` is.
//!
//! The rule this crate exists to hold: **an MCP `close` goes through the same
//! evidence gate as the CLI.** The anti-pattern refused by example is
//! `mcp-shrimp-task-manager`'s `verify_task`, which accepts a completion score
//! the calling agent supplies and auto-completes at 80 or above. That is an agent
//! grading its own homework, and it is exactly the failure aplan exists to
//! reject. There is no score parameter in [`tools::close`], and no path that
//! could accept one: a caller supplies *evidence*, a citation something else
//! resolves.
//!
//! The tool surface is deliberately small ([`tools::TOOLS`]), and pinned by a
//! test so it cannot grow silently.

pub mod server;
pub mod tools;

pub use server::{AplanServer, serve};
pub use tools::{TOOLS, ToolError, ToolSpec};
