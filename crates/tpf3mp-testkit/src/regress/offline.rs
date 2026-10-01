//! Plays a scenario without a room: a sequencer in place of the server
//! orders each action a fixed number of steps after its actor sends it.
//! It checks a scenario's own expectations in milliseconds; the room
//! ([`super::run`]) checks that the whole stack plays it the same way.

use std::{collections::BTreeMap, sync::Arc};

use tpf3mp_proto::{Event, EventBody, FixedBytes, Platform, PlayerId, Text};

use super::{
    model::ModelWorld,
    replica::ReplicaReport,
    script::{Cursor, Scenario, ScriptError},
};

pub struct OfflinePlan {
    pub scenario: Arc<Scenario>,
    pub replicas: usize,
    pub world_seed: u64,
    /// Steps from sending an action to the step it is applied before.
    pub delay: u64,
    pub checkpoint_interval: u64,
    /// This replica's world deviates once at this step.
    pub drift: Option<(usize, u64)>,
    /// Give up after this many steps.
    pub max_steps: u64,
}

impl OfflinePlan {
    pub fn new(scenario: Arc<Scenario>) -> Self {
        Self {
            replicas: scenario.players(2),
            scenario,
            world_seed: 1,
            delay: 3,
            checkpoint_interval: 25,
            drift: None,
            max_steps: 1_000_000,
        }
    }
}

struct Replica {
    player: PlayerId,
    world: ModelWorld,
    cursor: Cursor,
    ran: u64,
    sent: usize,
    done: bool,
}

/// Plays the plan and reports each replica as a room's game would.
pub fn play_offline(plan: &OfflinePlan) -> Result<Vec<ReplicaReport>, ScriptError> {
    let mut replicas = Vec::with_capacity(plan.replicas);
    for index in 0..plan.replicas {
        let mut world = ModelWorld::new(plan.world_seed);
        if let Some((replica, step)) = plan.drift
            && replica == index
        {
            world = world.with_drift(step);
        }
        let mut cursor = Cursor::new(plan.scenario.clone())?;
        cursor.start(&world, 0);
        let id = u8::try_from(index + 1).expect("few replicas");
        replicas.push(Replica {
            player: PlayerId(FixedBytes([id; 32])),
            world,
            cursor,
            ran: 0,
            sent: 0,
            done: false,
        });
    }
    let mut seq = 0;
    let mut queue: BTreeMap<u64, Vec<Event>> = BTreeMap::new();
    for replica in &replicas {
        seq += 1;
        queue.entry(1).or_default().push(Event {
            seq,
            step: 1,
            body: EventBody::PlayerJoined {
                player: replica.player,
                name: Text::new("bot").expect("short name"),
                platform: Platform::current(),
            },
        });
    }
    for step in 1..=plan.max_steps {
        let events = queue.remove(&step).unwrap_or_default();
        let mut sent = Vec::new();
        for replica in replicas.iter_mut().filter(|r| !r.done) {
            for event in &events {
                replica.world.apply(event);
                replica.cursor.on_event(&replica.world, event, replica.ran);
            }
            replica.world.step(step);
            replica.ran = step;
            replica.cursor.on_step(&replica.world, step);
            if replica
                .cursor
                .end_step(plan.checkpoint_interval)
                .is_some_and(|end| step >= end)
            {
                replica.done = true;
                continue;
            }
            if let Some((item, payload)) = replica.cursor.due(&replica.world, &replica.player) {
                replica.cursor.sent(item, 0);
                replica.sent += 1;
                sent.push((replica.player, payload));
            }
        }
        for (player, payload) in sent {
            seq += 1;
            let at = step + plan.delay.max(1);
            queue.entry(at).or_default().push(Event {
                seq,
                step: at,
                body: EventBody::Command {
                    player,
                    client_seq: seq,
                    payload,
                    seal: None,
                },
            });
        }
        if replicas.iter().all(|r| r.done) {
            break;
        }
    }
    Ok(replicas
        .into_iter()
        .map(|r| ReplicaReport {
            player: r.player,
            ran: r.ran,
            lanes: r.world.lanes(),
            observation: r.world.observe(),
            checks: r.cursor.checks().to_vec(),
            reached: r.cursor.index(),
            finished_at: r.cursor.finished_at(),
            ignored: r.world.ignored().to_vec(),
            diverged: Vec::new(),
            refused: Vec::new(),
            stalled: false,
            ended: false,
            sent: r.sent,
            lag: Vec::new(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regress::{library, run::judge};

    #[test]
    fn every_scenario_passes_offline() {
        for scenario in library::scenarios() {
            let reports = play_offline(&OfflinePlan::new(scenario.clone())).unwrap();
            let failures = judge(&scenario, &reports);
            assert!(failures.is_empty(), "{}: {failures:#?}", scenario.name);
            assert!(reports.iter().all(|r| r.finished_at.is_some()));
        }
    }

    #[test]
    fn the_outcome_does_not_depend_on_the_delay() {
        for scenario in library::scenarios().into_iter().filter(|s| s.smoke) {
            let mut quick = OfflinePlan::new(scenario.clone());
            quick.delay = 1;
            let mut slow = OfflinePlan::new(scenario.clone());
            slow.delay = 40;
            for reports in [play_offline(&quick).unwrap(), play_offline(&slow).unwrap()] {
                assert!(judge(&scenario, &reports).is_empty(), "{}", scenario.name);
            }
        }
    }

    #[test]
    fn a_drifting_replica_is_caught() {
        let scenario = library::scenarios()
            .into_iter()
            .find(|s| s.name == "bus-line")
            .unwrap();
        let mut plan = OfflinePlan::new(scenario.clone());
        plan.drift = Some((1, 400));
        let failures = judge(&scenario, &play_offline(&plan).unwrap());
        assert!(
            failures.iter().any(|f| f.contains("differs from r0")),
            "{failures:#?}"
        );
    }

    #[test]
    fn each_refusal_is_refused_for_its_own_reason() {
        let scenario = library::scenarios()
            .into_iter()
            .find(|s| s.name == "refusals")
            .unwrap();
        let reports = play_offline(&OfflinePlan::new(scenario)).unwrap();
        let reasons: Vec<&str> = reports[0]
            .ignored
            .iter()
            .map(|(_, why)| why.as_str())
            .collect();
        let expected = [
            "is already built",
            "no Street node",
            "no station-4",
            "no vehicle-7",
            "no vehicle-7",
            "is not a depot",
            "needs 20000000",
            "an unnamed construction",
            "already stands there",
            "no edge",
            "a split at the end of an edge",
        ];
        assert_eq!(reasons.len(), expected.len(), "{reasons:#?}");
        for (reason, fragment) in reasons.iter().zip(expected) {
            assert!(reason.contains(fragment), "{reason:?} is not {fragment:?}");
        }
        assert_eq!(reports[0].ignored, reports[1].ignored);
    }

    #[test]
    fn a_script_whose_actor_never_acts_says_where_it_stopped() {
        let scenario = library::scenarios()
            .into_iter()
            .find(|s| s.name == "two-companies")
            .unwrap();
        let mut plan = OfflinePlan::new(scenario.clone());
        plan.replicas = 1;
        plan.max_steps = 5_000;
        let failures = judge(&scenario, &play_offline(&plan).unwrap());
        assert!(
            failures
                .iter()
                .any(|f| f.contains("stopped at item") && f.contains("(actor 1: AssignLine)")),
            "{failures:#?}"
        );
    }

    #[test]
    fn a_wrong_expectation_fails_on_every_replica() {
        let mut scenario = (*library::scenarios()[0]).clone();
        scenario.items.push(crate::regress::script::Item::Expect(
            crate::regress::script::Check::Lines(7),
        ));
        let scenario = Arc::new(scenario);
        let reports = play_offline(&OfflinePlan::new(scenario.clone())).unwrap();
        let failures = judge(&scenario, &reports);
        assert_eq!(failures.len(), reports.len(), "{failures:#?}");
        assert!(failures.iter().all(|f| f.contains("expected 7 lines")));
    }
}
