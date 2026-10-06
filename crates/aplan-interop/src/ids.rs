//! Deriving the next task id from the convention a plan already uses.
//!
//! `aplan` does not own the id format. This project writes `A1-046`, the first
//! pilot writes `AL1-07` and `ALR-03`, and both are right; an id is read by
//! people, so the plan that exists is the authority on what the next one should
//! look like. This module infers the pattern rather than imposing one, which is
//! also why it refuses on an empty plan instead of inventing a prefix: there is
//! nothing to infer from, and a guess would set a convention the repository then
//! has to live with.
//!
//! Sequential by default, per HLD Section 4. The documented cross-machine
//! footgun (two people generating the same next id offline) is caught by
//! `APLAN002` at the merge, which is why the derivation can be a convenience
//! rather than a coordination mechanism.
//!
//! A repository that cannot live with catching it there sets `id_strategy: ulid`
//! and gets [`mint`]'s other half: the counter is replaced by a ULID, so there
//! is nothing left for two authors to derive identically. The inference above
//! still runs, because the *prefix* is what makes an id readable and no two
//! authors collide over it (`A1-047`).

use std::collections::BTreeMap;

use aplan_core::{IdStrategy, Plan};

/// Why the next id could not be derived.
#[derive(Debug, thiserror::Error)]
pub enum IdError {
    /// The operating system would not supply randomness, under `id_strategy:
    /// ulid`.
    ///
    /// A refusal rather than a fallback: an id minted from a clock alone is not
    /// collision-free, and the whole reason a repository asks for this strategy
    /// is the collision. Failing loudly is the only answer that does not hand
    /// back exactly what was asked to be prevented.
    #[error(
        "could not read randomness from the operating system to mint a ULID ({os}); pass \
         --id <ID> to name this task yourself"
    )]
    Entropy {
        /// What the platform said. Not a `source`: `getrandom::Error` is
        /// `no_std` and does not implement `std::error::Error`, so it is
        /// reported by its own words rather than through a chain.
        os: getrandom::Error,
    },
    /// Nothing in the plan to infer a convention from.
    #[error(
        "no existing task ids to infer a convention from; pass --id <ID> for the first task, \
         and later tasks will follow whatever convention it sets"
    )]
    NoConvention,
    /// Tasks exist, but none whose id ends in a number.
    #[error(
        "no task id ends in a number, so there is no sequence to continue ({sample}); pass \
         --id <ID>"
    )]
    NoSequence {
        /// An existing id, so the message shows what was actually found.
        sample: String,
    },
}

/// One id family: everything before the trailing number, plus how that number is
/// written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Family {
    /// Everything before the trailing digits, e.g. `A1-`.
    stem: String,
    /// Digits in the highest-numbered member, so `A1-046` stays three wide.
    width: usize,
    /// Highest number seen.
    max: u32,
    /// How many ids are in this family, for choosing between rival conventions.
    count: usize,
    /// How many of them are not terminal, which is what makes a convention
    /// *current* rather than merely large (`A1-180`).
    ///
    /// Counted in the same pass as the rest, and that is the point: the first
    /// version asked the plan a second time and did not repeat this loop's
    /// exclusions, so a live *milestone* sharing a family's stem made that
    /// family read as current even when every task in it was closed. One fact
    /// read twice drifts, and here it took four lines to drift.
    live: usize,
}

/// Split an id into (stem, number, width). `A1-046` -> (`A1-`, 46, 3).
fn split(id: &str) -> Option<(&str, u32, usize)> {
    let digits_start = id
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_digit())
        .map(|(i, _)| i)
        .last()?;
    let digits = &id[digits_start..];
    let num = digits.parse().ok()?;
    Some((&id[..digits_start], num, digits.len()))
}

/// The families in a plan, keyed by stem.
///
/// Milestones are excluded by tag rather than by id shape. `M-v0.7` would
/// otherwise parse as the family `M-v0.` at number 7, and the next task would be
/// offered the id `M-v0.8`, which is a cut, not a task.
fn families(plan: &Plan) -> BTreeMap<String, Family> {
    let mut out: BTreeMap<String, Family> = BTreeMap::new();
    for lt in &plan.tasks {
        if lt.task.is_milestone() || contains_a_ulid(&lt.task.id) {
            continue;
        }
        let Some((stem, num, width)) = split(&lt.task.id) else {
            continue;
        };
        out.entry(stem.to_string())
            .and_modify(|f| {
                f.count += 1;
                f.live += usize::from(!lt.task.status.is_terminal());
                if num >= f.max {
                    f.max = num;
                    f.width = width;
                }
            })
            .or_insert(Family {
                stem: stem.to_string(),
                width,
                max: num,
                count: 1,
                live: usize::from(!lt.task.status.is_terminal()),
            });
    }
    out
}

/// The id shapes this plan currently mints, with the number left as `nn`
/// (`A1-038`, and `A1-180`).
///
/// For prose an adopter reads: `AL1-nn`, `WH2-nn`, `A1-nnn`. Empty where the
/// plan teaches no convention -- ULIDs, or ids that are words -- because an
/// invented example is worse than a sentence that does not mention ids.
///
/// The *width* is copied, so a plan of three-digit ids gets `nnn`: an example
/// that would not validate as an id in this plan teaches the wrong shape.
///
/// **A family counts when it still has live work, not when it has the most
/// tasks.** The first version took the family with the most tasks, which made a
/// generated *policy* block move whenever ordinary work changed a count: in
/// `p003`, `K4-*` and `K6-*` are equally real cohorts, and adding one K6 task
/// rewrote the contract from `K4-nnn` to `K6-nnn` in three places and failed the
/// drift gate, though no id policy had changed
/// (pre-public adoption evidence). `AgentContract`'s own doc says what that costs: a
/// block that moves when work happens fails its own check daily and is turned
/// off within a week.
///
/// Live work is the signal the count was groping for. A *stray* convention is
/// one nobody is adding to -- this repository's `A0-` has two tasks and both are
/// closed, against `A1-`'s 181 -- so it is described by its main one, which is
/// what the majority rule was for. Two cohorts both being worked are two
/// conventions, and saying so is the honest answer rather than picking the
/// bigger.
///
/// Where nothing is live the whole plan is history, and every family is
/// returned: there is no current convention to prefer, and a closed plan does
/// not change again.
#[must_use]
pub fn example_ids(plan: &Plan) -> Vec<String> {
    let all = families(plan);
    let current: Vec<&Family> = all.values().filter(|f| f.live > 0).collect();
    let chosen = if current.is_empty() {
        all.values().collect()
    } else {
        current
    };
    chosen
        .into_iter()
        .map(|f| format!("{}{}", f.stem, "n".repeat(f.width.max(1))))
        .collect()
}

/// The phase to use when the caller did not name one, given where the task is
/// being placed (`A1-108`).
///
/// A `--cut` is the better answer than a guess, and `add` had it in its hand:
/// adding to cut 1 of a plan whose highest phase is 6 produced `R6-09`,
/// `phase: 6`, a member of `M-s1`. `check` calls that clean -- the phase and the
/// release cut are different axes on purpose (ROADMAP section 10), so a member of
/// one cut may sit in any phase -- but `summary` then bills the effort to phase 6
/// and any repository whose ids encode the cut gets an id that misstates it, on
/// the first step its own instructions document (pre-public adoption evidence item
/// 22).
///
/// **Taken from the milestone, not from its members.** A cut whose members span
/// phases is legal and this model permits it deliberately, so deriving from the
/// members would mean picking one; the milestone carries a `phase` of its own and
/// is the cut's own statement about where it sits. An explicit `--phase` still
/// wins, because an author who says so means it.
#[must_use]
pub fn phase_for(plan: &Plan, cut: Option<&str>) -> u32 {
    let from_cut = cut.and_then(|id| {
        plan.tasks
            .iter()
            .find(|lt| lt.task.id == id && lt.task.is_milestone())
            .map(|lt| lt.task.phase)
    });
    from_cut.unwrap_or_else(|| default_phase(plan))
}

/// The phase to use when nothing better is known: the highest in the plan,
/// because new work joins the phase the project is in rather than reopening an
/// old one. An empty plan starts at 1.
#[must_use]
pub fn default_phase(plan: &Plan) -> u32 {
    plan.tasks
        .iter()
        .filter(|lt| !lt.task.is_milestone())
        .map(|lt| lt.task.phase)
        .max()
        .unwrap_or(1)
}

/// The id for a new task, under the strategy the repository configured
/// (`A1-047`).
///
/// The one entry point both surfaces call, so the CLI and the MCP tool cannot
/// mint by different rules: a view assembled twice is two answers that agree
/// until they do not (`A1-104`).
///
/// # Errors
///
/// Whatever [`next_id`] returns under `sequential`, and [`IdError::Entropy`]
/// under `ulid` when the platform has no randomness to give.
pub fn mint(plan: &Plan, phase: u32, strategy: IdStrategy) -> Result<String, IdError> {
    match strategy {
        IdStrategy::Sequential => next_id(plan, phase),
        IdStrategy::Ulid => ulid_id(plan, phase),
    }
}

/// A ULID, carrying the plan's own prefix when it has one (`A1-047`).
///
/// **The stem is kept, the counter is not.** The collision this strategy exists
/// to remove is the *derived counter*: two authors offline both compute
/// `A1-047`. Nothing about the prefix is derived in that sense -- both authors
/// write `A1-`, and the 80 random bits after it still cannot collide -- so
/// dropping it would cost the only readable part of the id for no gain. An id
/// is read by people, and `A1-01K9ZBW7YQ8N4TQGX2H6RC3PVD` at least says which
/// family and phase it belongs to.
///
/// A plan with nothing to infer from gets a bare ULID, which is also the one
/// case where this strategy answers where `sequential` must refuse: there is no
/// convention to continue, and this one does not need it.
fn ulid_id(plan: &Plan, phase: u32) -> Result<String, IdError> {
    // The stem `sequential` would have used, and only the stem: a plan with no
    // convention to infer raises there and mints a bare ULID here, which is the
    // one place the two strategies are allowed to differ.
    let stem = convention_for(plan, phase).map_or_else(|_| String::new(), |c| c.stem);
    Ok(format!("{stem}{}", ulid()?))
}

/// How long to wait before trying a minted id again, when another writer took it.
///
/// **Exponential with full jitter** (`A1-230`). Sequential ids serialise
/// creation: every writer derives the same next id from the same plan, so one
/// wins and the rest find the file already there. Measured at 24 concurrent
/// `add`, two succeeded. Retrying without a wait reproduces the thundering herd
/// -- the losers re-read together, mint the same id together and lose together
/// -- so the wait is randomised across the whole window rather than scaled by
/// attempt alone, which is what spreads a herd out.
///
/// Capped, because the window only has to exceed the time a competing write
/// takes: over five runs of 24 writers, all of them succeed every time, with
/// the slowest taking about 160ms and **ten** attempts at the worst. Ten rather
/// than the five a first reading suggests, because a loser that re-reads is
/// racing the writers who have since joined as well as the ones it lost to. The
/// cap is generous against that and the deadline above it is the real bound.
#[must_use]
pub fn race_backoff(attempt: u32) -> std::time::Duration {
    let mut bytes = [0u8; 8];
    let roll = getrandom::fill(&mut bytes)
        .is_ok()
        .then(|| u64::from_le_bytes(bytes));
    jittered(race_window(attempt), roll)
}

/// The window attempt `attempt` draws its wait from, in milliseconds.
///
/// 2, 4, 8 .. 32, then flat. `saturating_pow` rather than a shift so a large
/// attempt count cannot wrap into a tiny wait, and a floor of 2 so the caller
/// that counts from zero still waits: a one-millisecond window can only draw
/// zero, which is a spin rather than a backoff.
///
/// **The cap is a trade, and both of the clever answers were measured and lost**
/// (`A1-230`, audit). Raising it to 256ms turns 128 concurrent writers from 113
/// landing in 5.5s into all 128 in 2.8s, and costs the ordinary case: 48 writers
/// go from about 440ms to about 1,150ms. Scaling the window by how long the lost
/// attempt itself took is the principled version -- an attempt costs more
/// exactly when there is more contention, because it re-reads a plan more
/// writers have grown -- and it also lands all 128, but it waits a multiple of
/// that cost on the *first* retry, where the ordinary race resolves. Measured, 8
/// writers went from about 45ms to 180-395ms, and 24 from about 165ms to about
/// 565ms.
///
/// So this stays flat and small, because the case it is tuned for is the one
/// that happens: a handful of agents, resolving in one or two attempts. What a
/// fleet past about a hundred simultaneous writers gets instead is graceful
/// degradation -- measured at 113 of 128, each refusal naming the id it lost and
/// leaving a plan that loads -- and that is the documented limit rather than a
/// surprise.
const fn race_window(attempt: u32) -> u64 {
    // `if` rather than `min`/`clamp`, which are not const-stable at this
    // crate's MSRV.
    let doublings = if attempt < 5 { attempt } else { 5 };
    let window = 2u64.saturating_pow(doublings);
    if window < 2 { 2 } else { window }
}

/// `roll` spread across `window`, or the whole window when there was no roll.
///
/// Split out to be tested, because the failure it guards cannot be provoked:
/// **the fallback used to sleep zero.** It was written as `roll = window` and
/// then taken modulo the window, so a `getrandom` failure produced `window %
/// window`, which is a spin loop at every attempt rather than the "correct wait,
/// merely less well spread" its own comment claimed. Nothing caught it because
/// the arithmetic is only reachable when the operating system cannot produce
/// eight bytes of randomness (`A1-230`, audit).
const fn jittered(window: u64, roll: Option<u64>) -> std::time::Duration {
    let ms = match roll {
        // Full jitter: anywhere in the window, zero included. A herd is spread
        // by the *spread*, and a floor on each wait would only shift it.
        Some(roll) => roll % window,
        None => window,
    };
    std::time::Duration::from_millis(ms)
}

/// Crockford base32, the alphabet a ULID is written in: no `I`, `L`, `O` or `U`,
/// so a transcribed id cannot be misread as a digit or as profanity.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Whether an id holds something only [`ulid`] would have written.
///
/// Not "is a ULID": the question is whether this id's trailing digits are a
/// *counter* to continue. A ULID's last characters are random base32, so
/// `A1-01K9ZBW7YQ8N4TQGX2H6RC3PV9` would otherwise read as the family
/// `A1-01K9...RC3PV` at number 9, and the next sequential id would be minted
/// from it -- an id nobody typed, outvoting the convention the plan declares in
/// every other id it holds. A repository that switches strategies, or switches
/// back, gets that answer without this (`A1-047`).
///
/// Every window rather than the tail alone, because a hand-written `--id` may
/// put characters after the ULID as easily as before it, and either way what
/// follows is not a sequence.
fn contains_a_ulid(id: &str) -> bool {
    if !id.is_ascii() || id.len() < ULID_LEN {
        return false;
    }
    (0..=id.len() - ULID_LEN).any(|start| is_a_ulid(&id[start..start + ULID_LEN]))
}

/// Whether exactly these 26 characters are a ULID this module could have
/// written.
fn is_a_ulid(window: &str) -> bool {
    let mut value: u128 = 0;
    for b in window.bytes() {
        let Some(i) = CROCKFORD.iter().position(|c| *c == b.to_ascii_uppercase()) else {
            return false;
        };
        value = (value << 5) | i as u128;
    }
    // The timestamp, and not merely the alphabet. Crockford omits `I`, `L`, `O`
    // and `U`, so most English words are already excluded, but a 26-character id
    // that happens to avoid those four letters is not far-fetched in a
    // repository that names tasks rather than numbering them. A ULID leads with
    // 48 bits of millisecond timestamp, so requiring it to fall in this
    // millennium is the discriminator that costs nothing and answers the
    // question actually being asked (`A1-047`).
    let ms = (value >> 80) as u64;
    (MS_2020..MS_2100).contains(&ms)
}

/// 2020-01-01 and 2100-01-01 in milliseconds since the epoch: the window a
/// minted ULID's timestamp must fall in for its id to be read as one.
const MS_2020: u64 = 1_577_836_800_000;
const MS_2100: u64 = 4_102_444_800_000;

/// How many characters a ULID is: 128 bits at five bits a character.
const ULID_LEN: usize = 26;

/// One ULID: 48 bits of millisecond timestamp, then 80 random bits.
///
/// Written here rather than taken as a dependency because it is fifteen lines
/// against a published specification, and the specification is the part that
/// has to be right: the timestamp leads so ids sort by when they were minted,
/// and the randomness is what makes two of them, on two machines, in the same
/// millisecond, different.
///
/// # Errors
///
/// [`IdError::Entropy`] when the platform will not supply randomness.
fn ulid() -> Result<String, IdError> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    // The low 48 bits, which is what the format carries: it runs out in 10889 AD.
    let ms = ms & 0xFFFF_FFFF_FFFF;

    let mut entropy = [0u8; 10];
    getrandom::fill(&mut entropy).map_err(|os| IdError::Entropy { os })?;
    let mut value = ms << 80;
    for (i, byte) in entropy.iter().enumerate() {
        value |= u128::from(*byte) << (72 - i * 8);
    }

    // Most significant five bits first, so the timestamp leads and the string
    // sorts the way the number does.
    let mut out = String::with_capacity(ULID_LEN);
    for i in 0..ULID_LEN {
        let shift = 125 - 5 * i;
        let index = ((value >> shift) & 0x1F) as usize;
        out.push(char::from(CROCKFORD[index]));
    }
    Ok(out)
}

/// The next free id for `phase`, in the convention the plan already uses.
///
/// Prefers the family used by tasks *in that phase*, so a plan whose ids encode
/// the phase (`A1-`, `A2-`) keeps doing so. When the phase is new, the phase
/// digit in the busiest family's stem is substituted, which is the only
/// inference here that guesses: it is spelled out in the returned id, and the
/// caller can override with `--id`.
///
/// # Errors
///
/// [`IdError::NoConvention`] when the plan holds no non-milestone task, and
/// [`IdError::NoSequence`] when none of their ids ends in a number.
pub fn next_id(plan: &Plan, phase: u32) -> Result<String, IdError> {
    let Convention { stem, width, next } = convention_for(plan, phase)?;
    Ok(format!("{stem}{next:0width$}"))
}

/// The convention a new id in `phase` should follow: which stem, how wide its
/// number is written, and what that number would be.
///
/// One derivation, because two strategies read it (`A1-047`). `ulid` takes the
/// stem and discards the counter, and it must be the *same* stem `sequential`
/// would have used: a plan whose phase 2 is `B2-` and whose busiest family is
/// `A1-` got `B2-003` from one strategy and `A2-<ULID>` from the other, so
/// turning the key on silently renamed the family a phase belongs to. Found by
/// the audit of `A1-047`, in the strategy that had promised to keep the plan's
/// own prefix.
struct Convention {
    stem: String,
    width: usize,
    next: u32,
}

/// Busiest first, so one stray id cannot outvote the established convention,
/// and ties by stem, so the answer does not depend on iteration order.
fn ranked(mut families: Vec<&Family>) -> Vec<&Family> {
    families.sort_by(|a, b| b.count.cmp(&a.count).then(a.stem.cmp(&b.stem)));
    families
}

/// Derive it, or say why the plan does not have one.
fn convention_for(plan: &Plan, phase: u32) -> Result<Convention, IdError> {
    let in_phase: Vec<_> = plan
        .tasks
        .iter()
        .filter(|lt| !lt.task.is_milestone() && lt.task.phase == phase)
        .map(|lt| lt.task.id.clone())
        .collect();

    let all = families(plan);
    if all.is_empty() {
        let sample = plan
            .tasks
            .iter()
            .find(|lt| !lt.task.is_milestone())
            .map(|lt| lt.task.id.clone());
        return Err(match sample {
            Some(sample) => IdError::NoSequence { sample },
            None => IdError::NoConvention,
        });
    }

    // The family this phase already uses, if it uses one.
    let phase_families = ranked(
        all.values()
            .filter(|f| in_phase.iter().any(|id| id.starts_with(&f.stem)))
            .collect(),
    );
    if let Some(f) = phase_families.first() {
        return Ok(Convention {
            stem: f.stem.clone(),
            width: f.width,
            next: f.max + 1,
        });
    }

    // A phase with no tasks yet. Take the busiest family and substitute the phase
    // digit if its stem carries one (`A1-` -> `A2-`); otherwise reuse the stem as
    // it stands, since it evidently does not encode the phase (`ALR-`).
    let busiest = ranked(all.values().collect())[0];
    let stem = substitute_phase(&busiest.stem, phase);
    let next = if stem == busiest.stem {
        busiest.max + 1
    } else {
        1
    };
    Ok(Convention {
        stem,
        width: busiest.width,
        next,
    })
}

/// Replace a trailing phase number in a stem: `A1-` with phase 2 gives `A2-`.
/// A stem with no digits (`ALR-`) is returned unchanged.
fn substitute_phase(stem: &str, phase: u32) -> String {
    let trimmed = stem.trim_end_matches(['-', '_', '.']);
    let separator = &stem[trimmed.len()..];
    let letters = trimmed.trim_end_matches(|c: char| c.is_ascii_digit());
    if letters.len() == trimmed.len() {
        return stem.to_string();
    }
    format!("{letters}{phase}{separator}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aplan_core::Status;
    use aplan_core::load::LoadedTask;
    use aplan_core::task::Task;

    fn task(id: &str, phase: u32, milestone: bool) -> LoadedTask {
        LoadedTask {
            criteria: aplan_core::load::Criteria::default(),
            task: Task {
                id: id.to_string(),
                title: id.to_string(),
                phase,
                status: Status::Todo,
                owner: None,
                effort_days: Some(1.0),
                blocked_by: Vec::new(),
                prefer_after: Vec::new(),
                ratified_by: Vec::new(),
                contains: Vec::new(),
                evidence: Vec::new(),
                paths: Vec::new(),
                traces: Vec::new(),
                trace_fingerprints: std::collections::BTreeMap::new(),
                tags: if milestone {
                    vec!["milestone".into()]
                } else {
                    Vec::new()
                },
                revisit_at: None,
                version_impact: None,
                version_impact_reason: None,
                version: None,
            },
            path: std::path::PathBuf::from("plan/tasks/x.md"),
            fields: aplan_core::load::FieldLines::new(),
        }
    }

    fn plan(tasks: Vec<LoadedTask>) -> Plan {
        Plan {
            tasks,
            load_diagnostics: Vec::new(),
            unloadable: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn continues_the_sequence_and_keeps_its_width() {
        let p = plan(vec![
            task("A1-001", 1, false),
            task("A1-046", 1, false),
            task("A1-007", 1, false),
        ]);
        assert_eq!(next_id(&p, 1).unwrap(), "A1-047");
    }

    #[test]
    fn a_two_digit_convention_stays_two_digits() {
        // p0000004 writes `AL1-07`, and a plan that gets handed `AL1-008` has had
        // this tool impose a convention on it.
        let p = plan(vec![task("AL1-06", 1, false), task("AL1-07", 1, false)]);
        assert_eq!(next_id(&p, 1).unwrap(), "AL1-08");
    }

    #[test]
    fn a_family_that_has_grown_a_digit_keeps_the_wider_form() {
        // 099 -> 100 must not go back to 3 digits' worth of padding on 4 digits.
        let p = plan(vec![task("A1-099", 1, false), task("A1-100", 1, false)]);
        assert_eq!(next_id(&p, 1).unwrap(), "A1-101");
    }

    #[test]
    fn milestones_do_not_donate_their_id_shape() {
        // `M-v0.7` splits as the family `M-v0.` at 7. Left in, the next task in a
        // fresh phase could be offered `M-v0.8`, which is a cut.
        let p = plan(vec![
            task("M-v0.7", 1, true),
            task("M-v0.8", 1, true),
            task("A1-004", 1, false),
        ]);
        assert_eq!(next_id(&p, 1).unwrap(), "A1-005");
    }

    #[test]
    fn a_new_phase_takes_the_established_prefix_and_starts_at_one() {
        let p = plan(vec![task("A1-046", 1, false), task("A1-045", 1, false)]);
        assert_eq!(next_id(&p, 2).unwrap(), "A2-001");
    }

    #[test]
    fn a_stem_without_a_phase_digit_is_left_alone() {
        // `ALR-` is p0000004's cross-cutting family: the phase is not in the id,
        // so substituting one would invent a convention the repository does not use.
        let p = plan(vec![task("ALR-03", 1, false)]);
        assert_eq!(next_id(&p, 2).unwrap(), "ALR-04");
    }

    #[test]
    fn the_busiest_family_wins_a_tie_of_conventions() {
        // A plan with a stray id in another shape must not have that shape adopted.
        let mut tasks = vec![task("X9-1", 1, false)];
        for n in 1..=4 {
            tasks.push(task(&format!("A1-00{n}"), 1, false));
        }
        assert_eq!(next_id(&plan(tasks), 1).unwrap(), "A1-005");
    }

    #[test]
    fn an_empty_plan_refuses_rather_than_inventing_a_prefix() {
        let err = next_id(&plan(Vec::new()), 1).unwrap_err();
        assert!(matches!(err, IdError::NoConvention), "{err:?}");
        assert!(err.to_string().contains("--id"), "{err}");
    }

    #[test]
    fn ids_with_no_number_are_reported_with_one_of_them() {
        let p = plan(vec![task("groundwork", 1, false)]);
        let err = next_id(&p, 1).unwrap_err();
        assert!(err.to_string().contains("groundwork"), "{err}");
    }

    #[test]
    fn a_plan_of_only_milestones_has_no_convention_to_offer() {
        let p = plan(vec![task("M-v0.7", 1, true)]);
        assert!(matches!(next_id(&p, 1).unwrap_err(), IdError::NoConvention));
    }

    /// A cut whose members span phases is legal, so the phase is taken from the
    /// milestone rather than from its members: there is nothing to pick
    /// (`A1-108`).
    #[test]
    fn the_cut_decides_the_phase_and_the_milestone_decides_the_cut() {
        let mut m = task("M-s1", 1, true);
        m.task.blocked_by = vec!["R1-01".into(), "R6-01".into()];
        // Its members disagree with each other and with the milestone.
        let p = plan(vec![task("R1-01", 1, false), task("R6-01", 6, false), m]);

        assert_eq!(phase_for(&p, Some("M-s1")), 1, "the cut's own phase");
        assert_eq!(
            phase_for(&p, None),
            6,
            "with no cut, the highest in use is still the best guess"
        );
        // An id that names no milestone is not a placement, so it falls back
        // rather than inventing a phase: `cut::check_target` is what refuses it,
        // and it runs before anything is written.
        assert_eq!(phase_for(&p, Some("M-nope")), 6);
        assert_eq!(phase_for(&p, Some("R1-01")), 6, "a task is not a cut");
    }

    #[test]
    fn the_default_phase_is_the_highest_in_use() {
        let p = plan(vec![task("A0-001", 0, false), task("A1-001", 1, false)]);
        assert_eq!(default_phase(&p), 1);
        // A milestone must not drag the default into a phase no work is in.
        let p = plan(vec![task("A1-001", 1, false), task("M-v9", 9, true)]);
        assert_eq!(default_phase(&p), 1);
        assert_eq!(default_phase(&plan(Vec::new())), 1);
    }

    #[test]
    fn splitting_handles_the_shapes_that_occur() {
        assert_eq!(split("A1-046"), Some(("A1-", 46, 3)));
        assert_eq!(split("AL1-07"), Some(("AL1-", 7, 2)));
        assert_eq!(split("T-1"), Some(("T-", 1, 1)));
        assert_eq!(split("plain"), None);
        assert_eq!(split(""), None);
    }

    // --- `id_strategy: ulid` (`A1-047`) -------------------------------------

    /// The default strategy is the derivation this module has always done, to
    /// the character. A new key that changed what every existing plan gets would
    /// be a migration wearing a config key's clothes.
    #[test]
    fn sequential_mints_exactly_what_it_did_before_the_strategy_existed() {
        let p = plan(vec![task("A1-046", 1, false)]);
        assert_eq!(
            mint(&p, 1, IdStrategy::Sequential).expect("id"),
            next_id(&p, 1).expect("id")
        );
        assert_eq!(mint(&p, 1, IdStrategy::default()).expect("id"), "A1-047");
    }

    /// The shape, stated as the properties an adopter is buying: the plan's own
    /// prefix, then 26 characters of Crockford base32.
    #[test]
    fn a_ulid_id_is_the_plans_stem_and_twenty_six_crockford_characters() {
        let p = plan(vec![task("A1-046", 1, false)]);
        let id = mint(&p, 1, IdStrategy::Ulid).expect("id");
        let tail = id.strip_prefix("A1-").expect("the plan's own stem: {id}");
        assert_eq!(tail.len(), ULID_LEN, "{id}");
        assert!(
            tail.bytes().all(|b| CROCKFORD.contains(&b)),
            "a character outside Crockford base32, which is what makes an id \
             transcribable: {id}"
        );
    }

    /// The whole point, as a property rather than as a claim: nothing about a
    /// minted id comes from the ids already in the plan, so two authors offline
    /// cannot derive the same one. Sequential minting fails this by design --
    /// it returns `A1-047` every time until somebody commits.
    #[test]
    fn nothing_minted_under_ulid_repeats_even_from_an_unchanged_plan() {
        let p = plan(vec![task("A1-046", 1, false)]);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2_000 {
            assert!(
                seen.insert(mint(&p, 1, IdStrategy::Ulid).expect("id")),
                "two ids from one unchanged plan collided, which is the failure \
                 this strategy exists to remove"
            );
        }
        // And the contrast, so this test cannot pass by minting being broken in
        // some way that also breaks sequential.
        assert_eq!(next_id(&p, 1).expect("id"), next_id(&p, 1).expect("id"));
    }

    /// A ULID leads with its timestamp, so ids sort into the order they were
    /// minted. Worth pinning: it is why the timestamp is there at all, and a
    /// re-ordering of the two halves would pass every other test here.
    #[test]
    fn a_later_id_sorts_after_an_earlier_one() {
        let p = plan(Vec::new());
        let first = mint(&p, 1, IdStrategy::Ulid).expect("id");
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second = mint(&p, 1, IdStrategy::Ulid).expect("id");
        assert!(first < second, "{first} did not sort before {second}");
    }

    /// The one case where this strategy answers and `sequential` cannot: there
    /// is no convention to continue, and this does not need one.
    #[test]
    fn an_empty_plan_gets_a_bare_ulid_where_sequential_must_refuse() {
        let p = plan(Vec::new());
        assert!(matches!(next_id(&p, 1), Err(IdError::NoConvention)));
        let id = mint(&p, 1, IdStrategy::Ulid).expect("id");
        assert_eq!(id.len(), ULID_LEN, "a stem was invented: {id}");
    }

    /// A plan holding ULIDs must not have one read as a counter to continue.
    ///
    /// The trap is concrete: a ULID ending in a digit splits into a stem and a
    /// number like any other id, and the family it invents would be the busiest
    /// one in a plan that had switched. Then a repository switching back gets
    /// `<26 characters>-1` as its next "sequential" id.
    #[test]
    fn a_ulid_in_the_plan_is_not_a_sequence_to_continue() {
        let p = plan(Vec::new());
        let minted = mint(&p, 1, IdStrategy::Ulid).expect("id");
        let p = plan(vec![
            task("A1-046", 1, false),
            task(&minted, 1, false),
            task(&format!("{minted}9"), 1, false),
        ]);
        assert_eq!(
            next_id(&p, 1).expect("id"),
            "A1-047",
            "the plan's convention lost to an id nobody typed"
        );
    }

    /// And the recogniser itself, in both directions, because a false positive
    /// silently drops a repository's own convention.
    #[test]
    fn a_ulid_is_recognised_and_an_ordinary_id_is_not() {
        let minted = mint(&plan(Vec::new()), 1, IdStrategy::Ulid).expect("id");
        assert!(contains_a_ulid(&minted));
        assert!(contains_a_ulid(&format!("A1-{minted}")));
        assert!(contains_a_ulid(&format!("A1-{minted}9")));

        for ordinary in [
            "A1-047",
            "AL1-07",
            "",
            // 26 characters, entirely inside the alphabet, and not a ULID: the
            // timestamp check is what refuses it.
            "ZZZZZZZZZZZZZZZZZZZZZZZZZZ",
            "00000000000000000000000000",
            // A long, wordy id of the kind a repository that names its tasks
            // writes.
            "refactor-the-authentication-layer",
        ] {
            assert!(!contains_a_ulid(ordinary), "{ordinary} was read as a ULID");
        }
    }

    /// Turning the key on must not rename the family a phase belongs to
    /// (`A1-047`, found by its audit).
    ///
    /// The measured case: phase 2 is `B2-`, the busiest family in the plan is
    /// `A1-`, and `ulid` took the busiest one and substituted the phase digit.
    /// So `sequential` wrote `B2-003` and `ulid` wrote `A2-<ULID>` -- a prefix
    /// the plan had never used, from the strategy whose documented promise is
    /// that the plan's own prefix is kept.
    ///
    /// Stated as a property over the shapes this module claims to infer, rather
    /// than as the one case: whatever stem `sequential` chooses, `ulid` carries
    /// the same one.
    #[test]
    fn both_strategies_agree_on_the_stem_they_carry() {
        let cases: Vec<(&str, Vec<LoadedTask>, u32)> = vec![
            (
                "the phase's own family beats the busiest one",
                vec![
                    task("A1-001", 1, false),
                    task("A1-002", 1, false),
                    task("A1-003", 1, false),
                    task("B2-001", 2, false),
                    task("B2-002", 2, false),
                ],
                2,
            ),
            (
                "a phase with no tasks yet substitutes the digit",
                vec![task("A1-001", 1, false), task("A1-002", 1, false)],
                3,
            ),
            (
                "a stem that does not encode the phase is reused",
                vec![task("ALR-01", 1, false), task("ALR-02", 1, false)],
                4,
            ),
            (
                "two families of equal size resolve the same way for both",
                vec![task("B1-001", 1, false), task("A1-001", 1, false)],
                1,
            ),
            (
                "milestones do not lend their stem to either",
                vec![task("A1-001", 1, false), task("M-v0.7", 1, true)],
                1,
            ),
        ];

        for (what, tasks, phase) in cases {
            let p = plan(tasks);
            let sequential = next_id(&p, phase).expect("an id");
            let with_ulid = mint(&p, phase, IdStrategy::Ulid).expect("an id");
            let stem = sequential.trim_end_matches(|c: char| c.is_ascii_digit());
            assert!(
                with_ulid.starts_with(stem),
                "{what}: `sequential` chose `{stem}` and `ulid` wrote `{with_ulid}`"
            );
            assert_eq!(
                with_ulid.len(),
                stem.len() + ULID_LEN,
                "{what}: `{with_ulid}` is not that stem followed by a ULID"
            );
        }
    }

    /// A multi-byte id must not panic the recogniser by slicing mid-character.
    #[test]
    fn a_non_ascii_id_is_not_a_panic() {
        assert!(!contains_a_ulid("A1-046-cafe\u{e9}"));
        assert!(!contains_a_ulid(&"\u{e9}".repeat(30)));
    }
}

#[cfg(test)]
mod backoff {
    use super::*;

    /// **A backoff that cannot draw randomness still waits** (`A1-230`, audit).
    ///
    /// The defect this pins: the fallback was the window taken modulo the
    /// window, so it slept zero at every attempt, and the losers of a race
    /// would spin against each other for the whole five-second deadline
    /// instead of spreading out. It is unreachable in normal operation, which
    /// is exactly why it needs a test rather than a reading.
    ///
    /// Proof of `ids.sequential_survives_concurrent_writers`: the wait that
    /// makes re-minting spread a herd out instead of re-colliding with it.
    #[test]
    fn a_failed_draw_waits_the_whole_window() {
        for attempt in 0..8 {
            let window = race_window(attempt);
            assert_eq!(
                jittered(window, None),
                std::time::Duration::from_millis(window),
                "attempt {attempt} falls back to a {window}ms window"
            );
            assert!(
                !jittered(window, None).is_zero(),
                "attempt {attempt} falls back to a spin rather than a wait"
            );
        }
    }

    /// **Every attempt can wait, including the first** (`A1-230`, audit).
    ///
    /// A one-millisecond window can only draw zero, so an attempt counted from
    /// zero would spin however good the randomness was. The window floor is 2.
    #[test]
    fn no_attempt_has_a_window_it_cannot_jitter_in() {
        for attempt in 0..8 {
            assert!(
                race_window(attempt) >= 2,
                "attempt {attempt} draws from a window of {}, which can only be zero",
                race_window(attempt)
            );
        }
    }

    /// **The wait stays inside its window, and the window stays capped**
    /// (`A1-230`).
    ///
    /// The cap is what bounds the worst case a writer pays once a herd has
    /// spread out; the deadline above it is the real bound, and a window that
    /// grew without limit would overshoot it in one sleep.
    #[test]
    fn a_drawn_wait_is_inside_its_window_and_the_window_is_capped() {
        for attempt in 0..40 {
            let window = race_window(attempt);
            assert!(window <= 32, "attempt {attempt} uncapped at {window}ms");
            for roll in [0, 1, 7, u64::MAX / 2, u64::MAX] {
                let waited = jittered(window, Some(roll)).as_millis();
                assert!(
                    waited < u128::from(window),
                    "attempt {attempt} with roll {roll} waited {waited}ms, outside its \
                     {window}ms window"
                );
            }
        }
    }

    /// **The window really grows** (`A1-230`).
    ///
    /// Exponential is the half that spreads a herd that keeps colliding: if
    /// every attempt drew from the same window, twenty-four writers would
    /// re-collide at the same rate however many times they tried.
    #[test]
    fn the_window_doubles_until_it_caps() {
        let windows: Vec<u64> = (0..8).map(race_window).collect();
        assert_eq!(windows, vec![2, 2, 4, 8, 16, 32, 32, 32], "{windows:?}");
    }
}
