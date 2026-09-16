//! The body `aplan add` writes, and the prompts inside it.
//!
//! It lives here rather than in `aplan-interop` because two things need it and
//! they are on opposite sides of the dependency: `emit` writes it, and a rule
//! reads it (`A1-271`). A rule that carried its own copy would stop firing the
//! day somebody reworded the scaffold, silently, which is the failure mode the
//! rule exists to catch one level down.
//!
//! **Prompts, not content.** Every line is a question only the author can
//! answer, and an `aplan` that answered them would be generating a plan rather
//! than recording one.
//!
//! There are **two** criterion prompts, not one. `add` asks the author to
//! *state* what done means for this *task*; the import paths ask them to
//! *restate* it for this *item*, because an imported item arrived with words of
//! its own. A comment here used to claim they were the same line, which was
//! false, and the rule built on that comment knew only the first -- so a task
//! created by `import` and closed with its prompt ticked made the same false
//! claim and nothing reported it, on the path that creates tasks in bulk
//! (`A1-271`'s audit).

/// The body written into a task `add` creates.
pub const ADD_BODY: &str = "## Why\n\n\
     (Why this task exists, and what is different once it is done.)\n\n\
     ## Acceptance criteria\n\n\
     - [ ] state what done means for this task, in this repository's terms\n";

/// The criterion the **import** paths write, without its checkbox.
///
/// A different sentence from [`ADD_BODY`]'s -- *restate*, and *item* rather than
/// *task* -- because an imported item came from somewhere and its own words are
/// already in the body. It is the same claim when ticked, and `A1-271`'s audit
/// found the rule knew only the `add` one: an imported task closed with its
/// prompt ticked said the same false thing and nothing reported it, on the path
/// that creates tasks in bulk.
pub const IMPORT_CRITERION: &str =
    "restate what done means for this item, in this repository's terms";

/// Every criterion prompt this tool writes, so a reader of one is a reader of
/// all of them.
///
/// The rule that refuses to accept a prompt back as an answer walks this, and
/// `every_prompt_this_tool_writes_is_declared_here` fails if a writer grows a
/// sentence that is not in it.
#[must_use]
pub fn criterion_prompts() -> [&'static str; 2] {
    [criterion_prompt(), IMPORT_CRITERION]
}

/// The criterion [`ADD_BODY`] writes, without its checkbox.
///
/// Derived from the constant rather than written beside it, so the rule that
/// recognises it cannot drift from what `add` produces: rewording the scaffold
/// changes both at once, and the test below fails if the shape it looks for
/// stops existing.
#[must_use]
pub fn criterion_prompt() -> &'static str {
    ADD_BODY
        .lines()
        .find_map(|l| l.strip_prefix("- [ ] "))
        .expect("ADD_BODY states one unticked criterion")
}

/// The prose [`ADD_BODY`] writes under `## Why`, derived the same way.
#[must_use]
pub fn why_prompt() -> &'static str {
    ADD_BODY
        .lines()
        .find(|l| l.starts_with('(') && l.ends_with(')'))
        .expect("ADD_BODY states one `Why` prompt")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both accessors find something, or the rule reading them matches text
    /// nothing produces and goes quiet without saying so (`A1-271`).
    #[test]
    fn the_prompts_are_derived_from_the_body_and_both_resolve() {
        assert_eq!(
            criterion_prompt(),
            "state what done means for this task, in this repository's terms"
        );
        assert_eq!(
            why_prompt(),
            "(Why this task exists, and what is different once it is done.)"
        );
        // And they are *in* it, which is the property that keeps them honest.
        assert!(ADD_BODY.contains(criterion_prompt()));
        assert!(ADD_BODY.contains(why_prompt()));
    }
}
