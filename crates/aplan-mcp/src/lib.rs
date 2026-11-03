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

/// Everything the server needs that is not the plan itself.
///
/// One carrier rather than a growing parameter list: `deny_warnings` arrived with
/// `A1-101` and the verify settings with `A1-102`, and a fourth loose argument is
/// how one gets passed wrongly. Each field is resolved by `aplan mcp` from
/// `.aplan.yml`, from the *plan's* directory and never the caller's, which is the
/// defect `A1-100` shipped with.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// Which rules run, as `check` sees them.
    pub options: aplan_core::Options,
    /// The repository's verdict policy: whether a warning blocks, and which rules
    /// it hears without being stopped by (`A1-112`). A verdict policy, not a
    /// validation one.
    pub verdict: aplan_core::Verdict,
    /// How evidence is resolved. `execute` stays off: running a cited test
    /// because an agent asked is a different trust question from reading its
    /// history, and nobody has asked for it (`A1-102`). Configured discovery
    /// and checker commands may still run.
    pub verify: aplan_evidence::VerifyOptions,
    /// Where git lives, which is the repository the plan sits in and not the
    /// directory the client happened to start the server from.
    pub repo_root: std::path::PathBuf,
    /// How `aplan_add` mints an id (`A1-047`). Travels with the policy for the
    /// reason the verdict does: a repository that configured `ulid` because two
    /// authors collide offline has an *agent* as one of those authors, and a
    /// surface that quietly kept deriving the counter would be the collision.
    pub id_strategy: aplan_core::IdStrategy,
}

pub use server::{AplanServer, serve, serve_unconfigured};
pub use tools::{TOOLS, ToolError, ToolSpec};
