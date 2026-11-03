//! Bounded deterministic orchestration for configured evidence providers.
//!
//! Commands remain synchronous and own their process groups. This coordinator
//! overlaps independent commands while reserving one aggregate weighted budget
//! and every named capacity-one resource atomically. Completion order is never
//! result order: callers receive one slot per configured runner.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;

/// One configured runner's reservation for one provider stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Job {
    pub index: usize,
    pub weight: u32,
    pub resources: BTreeSet<String>,
}

/// Coordinator boundary events, delivered on the coordinator thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    Started { index: usize },
    Finished { index: usize },
    Cancelled { index: usize },
    Panicked { index: usize },
}

/// One job's terminal scheduler state.
#[derive(Debug)]
pub(crate) enum Outcome<T> {
    Completed(T),
    Cancelled,
    Panicked,
}

#[derive(Debug)]
struct Completed<T> {
    job: Job,
    outcome: Result<T, ()>,
}

/// Run finite jobs under one weighted capacity and named resource set.
///
/// Queue selection is deterministic. A blocked job may be bypassed only by a
/// later job that fits the currently free capacity and resources; because the
/// queue is finite and finished reservations are always released, the oldest
/// compatible job is selected next and cannot starve. Cancellation removes the
/// remaining queue, starts nothing else and drains every active completion.
pub(crate) fn run<T, Work, Cancel, Observe>(
    jobs: Vec<Job>,
    capacity: u32,
    work: Work,
    cancelled: Cancel,
    mut observe: Observe,
) -> BTreeMap<usize, Outcome<T>>
where
    T: Send,
    Work: Fn(usize) -> T + Sync,
    Cancel: Fn() -> bool,
    Observe: FnMut(Event, Option<&T>),
{
    assert!(capacity > 0, "validated scheduler capacity is nonzero");
    let mut indices = BTreeSet::new();
    for job in &jobs {
        assert!(
            job.weight > 0 && job.weight <= capacity,
            "validated scheduler weights fit aggregate capacity"
        );
        assert!(
            indices.insert(job.index),
            "scheduler job indices are unique"
        );
    }
    let mut results = BTreeMap::new();
    let mut queue = jobs.into_iter().collect::<VecDeque<_>>();
    let mut active_weight = 0u32;
    let mut active_resources = BTreeSet::new();
    let mut active = 0usize;
    let mut stopping = false;
    let (finished_tx, finished_rx) = mpsc::channel();

    std::thread::scope(|scope| {
        loop {
            if !stopping && cancelled() {
                stopping = true;
                for job in queue.drain(..) {
                    observe(Event::Cancelled { index: job.index }, None);
                    assert!(
                        results.insert(job.index, Outcome::Cancelled).is_none(),
                        "scheduler job indices are unique"
                    );
                }
            }

            while !stopping {
                if cancelled() {
                    stopping = true;
                    for job in queue.drain(..) {
                        observe(Event::Cancelled { index: job.index }, None);
                        assert!(
                            results.insert(job.index, Outcome::Cancelled).is_none(),
                            "scheduler job indices are unique"
                        );
                    }
                    break;
                }
                let available = capacity.saturating_sub(active_weight);
                let Some(position) = queue.iter().position(|job| {
                    job.weight <= available && job.resources.is_disjoint(&active_resources)
                }) else {
                    break;
                };
                let job = queue
                    .remove(position)
                    .expect("a selected scheduler job still exists");
                active_weight = active_weight
                    .checked_add(job.weight)
                    .expect("validated weights fit aggregate capacity");
                active_resources.extend(job.resources.iter().cloned());
                active += 1;
                let index = job.index;
                let tx = finished_tx.clone();
                let work = &work;
                scope.spawn(move || {
                    let outcome =
                        catch_unwind(AssertUnwindSafe(|| work(job.index))).map_err(|_| ());
                    let _ = tx.send(Completed { job, outcome });
                });
                // The worker must exist before an observer is allowed to wait
                // synchronously for something that worker opens (for example,
                // an event-driven FIFO fixture). Reservations are already held.
                observe(Event::Started { index }, None);
            }

            if active == 0 {
                break;
            }

            let completed = finished_rx
                .recv()
                .expect("an active scheduler worker retains the completion channel");
            active -= 1;
            active_weight = active_weight
                .checked_sub(completed.job.weight)
                .expect("a completed job held its declared weight");
            for resource in &completed.job.resources {
                let removed = active_resources.remove(resource);
                debug_assert!(removed, "a completed job held every declared resource");
            }
            let index = completed.job.index;
            if let Ok(value) = completed.outcome {
                observe(Event::Finished { index }, Some(&value));
                assert!(
                    results.insert(index, Outcome::Completed(value)).is_none(),
                    "scheduler job indices are unique"
                );
            } else {
                observe(Event::Panicked { index }, None);
                assert!(
                    results.insert(index, Outcome::Panicked).is_none(),
                    "scheduler job indices are unique"
                );
            }
        }
    });

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};

    fn resources(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn weighted_capacity_exclusion_and_fifo_progress_are_event_driven() {
        let jobs = vec![
            Job {
                index: 0,
                weight: 2,
                resources: resources(&["database"]),
            },
            Job {
                index: 1,
                weight: 2,
                resources: resources(&["database"]),
            },
            Job {
                index: 2,
                weight: 1,
                resources: BTreeSet::new(),
            },
        ];
        let (release_tx, release_rx) = (0..3)
            .map(|_| mpsc::channel())
            .unzip::<_, _, Vec<_>, Vec<_>>();
        let release_rx = Arc::new(release_rx.into_iter().map(Mutex::new).collect::<Vec<_>>());
        let mut starts = Vec::new();
        let mut active_weight = 0u32;
        let mut maximum_weight = 0u32;
        let mut database_active = false;
        let outcomes = run(
            jobs,
            3,
            {
                let release_rx = Arc::clone(&release_rx);
                move |index| {
                    release_rx[index]
                        .lock()
                        .expect("release gate")
                        .recv()
                        .expect("coordinator releases every start");
                    index
                }
            },
            || false,
            |event, _| match event {
                Event::Started { index } => {
                    starts.push(index);
                    let weight = if index == 2 { 1 } else { 2 };
                    active_weight += weight;
                    maximum_weight = maximum_weight.max(active_weight);
                    if index < 2 {
                        assert!(!database_active, "exclusive jobs overlapped");
                        database_active = true;
                    }
                    if starts.len() == 2 {
                        assert_eq!(starts, [0, 2], "a blocked job prevented useful work");
                        release_tx[0].send(()).expect("release first");
                        release_tx[2].send(()).expect("release independent");
                    } else if index == 1 {
                        release_tx[1].send(()).expect("release waiter");
                    }
                }
                Event::Finished { index } => {
                    active_weight -= if index == 2 { 1 } else { 2 };
                    if index < 2 {
                        database_active = false;
                    }
                }
                Event::Cancelled { .. } | Event::Panicked { .. } => {
                    panic!("unexpected terminal state: {event:?}")
                }
            },
        );
        assert_eq!(starts, [0, 2, 1]);
        assert_eq!(maximum_weight, 3);
        assert_eq!(active_weight, 0);
        assert!(outcomes.into_iter().all(
            |(index, outcome)| matches!(outcome, Outcome::Completed(value) if value == index)
        ));
    }

    #[test]
    fn cancellation_starts_nothing_new_and_drains_the_active_worker() {
        let jobs = (0..3)
            .map(|index| Job {
                index,
                weight: 1,
                resources: BTreeSet::new(),
            })
            .collect();
        let stopping = Arc::new(AtomicBool::new(false));
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let mut events = Vec::new();
        let outcomes = run(
            jobs,
            1,
            |_| {
                release_rx
                    .lock()
                    .expect("release gate")
                    .recv()
                    .expect("active worker is released");
                7
            },
            {
                let stopping = Arc::clone(&stopping);
                move || stopping.load(Ordering::Acquire)
            },
            |event, _| {
                events.push(event);
                if event == (Event::Started { index: 0 }) {
                    stopping.store(true, Ordering::Release);
                    release_tx.send(()).expect("release active worker");
                }
            },
        );
        assert_eq!(
            events,
            [
                Event::Started { index: 0 },
                Event::Cancelled { index: 1 },
                Event::Cancelled { index: 2 },
                Event::Finished { index: 0 },
            ]
        );
        assert!(matches!(outcomes[&0], Outcome::Completed(7)));
        assert!(matches!(outcomes[&1], Outcome::Cancelled));
        assert!(matches!(outcomes[&2], Outcome::Cancelled));
    }

    #[test]
    fn cancellation_before_admission_rejects_every_job_without_starting_work() {
        let jobs = (0..2)
            .map(|index| Job {
                index,
                weight: 1,
                resources: BTreeSet::new(),
            })
            .collect();
        let mut events = Vec::new();
        let outcomes = run(
            jobs,
            1,
            std::convert::identity,
            || true,
            |event, completed| {
                assert!(completed.is_none());
                events.push(event);
            },
        );

        assert_eq!(
            events,
            [Event::Cancelled { index: 0 }, Event::Cancelled { index: 1 },]
        );
        assert!(
            outcomes
                .values()
                .all(|outcome| matches!(outcome, Outcome::Cancelled))
        );
    }

    #[test]
    fn one_worker_panic_does_not_suppress_an_unrelated_job() {
        let jobs = (0..2)
            .map(|index| Job {
                index,
                weight: 1,
                resources: BTreeSet::new(),
            })
            .collect();
        let outcomes = run(
            jobs,
            2,
            |index| {
                assert_ne!(index, 0, "controlled worker fault");
                index
            },
            || false,
            |_, _| {},
        );
        assert!(matches!(outcomes[&0], Outcome::Panicked));
        assert!(matches!(outcomes[&1], Outcome::Completed(1)));
    }
}
