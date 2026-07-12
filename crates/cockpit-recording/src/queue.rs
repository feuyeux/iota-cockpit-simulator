use std::collections::VecDeque;

use cockpit_simulation_core::StepRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingQueuePolicy {
    PauseRun,
    FailRun,
    Drop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingQueueOutcome {
    Enqueued,
    Paused,
    Failed,
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingQueueHealth {
    pub capacity: usize,
    pub depth: usize,
    pub enqueued: u64,
    pub dropped: u64,
    pub rejected: u64,
}

#[derive(Debug)]
pub struct RecordingQueue {
    capacity: usize,
    policy: RecordingQueuePolicy,
    entries: VecDeque<StepRecord>,
    enqueued: u64,
    dropped: u64,
    rejected: u64,
}

impl RecordingQueue {
    pub fn new(capacity: usize, policy: RecordingQueuePolicy) -> Self {
        assert!(
            capacity > 0,
            "recording queue capacity must be greater than zero"
        );
        Self {
            capacity,
            policy,
            entries: VecDeque::with_capacity(capacity),
            enqueued: 0,
            dropped: 0,
            rejected: 0,
        }
    }

    pub fn push(&mut self, step: StepRecord) -> RecordingQueueOutcome {
        if self.entries.len() < self.capacity {
            self.entries.push_back(step);
            self.enqueued += 1;
            return RecordingQueueOutcome::Enqueued;
        }
        match self.policy {
            RecordingQueuePolicy::Drop => {
                self.dropped += 1;
                RecordingQueueOutcome::Dropped
            }
            RecordingQueuePolicy::PauseRun => {
                self.rejected += 1;
                RecordingQueueOutcome::Paused
            }
            RecordingQueuePolicy::FailRun => {
                self.rejected += 1;
                RecordingQueueOutcome::Failed
            }
        }
    }

    pub fn pop(&mut self) -> Option<StepRecord> {
        self.entries.pop_front()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = StepRecord> + '_ {
        self.entries.drain(..)
    }

    pub fn health(&self) -> RecordingQueueHealth {
        RecordingQueueHealth {
            capacity: self.capacity,
            depth: self.entries.len(),
            enqueued: self.enqueued,
            dropped: self.dropped,
            rejected: self.rejected,
        }
    }

    pub fn policy(&self) -> RecordingQueuePolicy {
        self.policy
    }

    pub fn depth(&self) -> usize {
        self.entries.len()
    }
}

/// An asynchronous recording sink whose consumer can lag behind the producer.
///
/// The synchronous handler path drains the queue immediately after every push,
/// so sustained overload never triggers there. This sink models a slow async
/// store consumer: `push` enqueues under the bounded policy, while `drain_one`
/// represents a single unit of consumer progress. When the producer outpaces
/// the consumer, the bounded overflow policy (`Paused`/`Failed`/`Dropped`)
/// is exercised for real.
#[derive(Debug)]
pub struct AsyncRecordingSink {
    queue: RecordingQueue,
    committed: Vec<StepRecord>,
}

impl AsyncRecordingSink {
    pub fn new(capacity: usize, policy: RecordingQueuePolicy) -> Self {
        Self {
            queue: RecordingQueue::new(capacity, policy),
            committed: Vec::new(),
        }
    }

    /// Enqueue a step for asynchronous persistence, returning the bounded
    /// overflow outcome.
    pub fn push(&mut self, step: StepRecord) -> RecordingQueueOutcome {
        self.queue.push(step)
    }

    /// Make one unit of consumer progress by committing the oldest queued step.
    /// Returns `true` if a step was committed.
    pub fn drain_one(&mut self) -> bool {
        match self.queue.pop() {
            Some(step) => {
                self.committed.push(step);
                true
            }
            None => false,
        }
    }

    /// Drain all currently queued steps (consumer fully catches up).
    pub fn drain_all(&mut self) {
        while self.drain_one() {}
    }

    pub fn committed(&self) -> &[StepRecord] {
        &self.committed
    }

    pub fn health(&self) -> RecordingQueueHealth {
        self.queue.health()
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordingQueue, RecordingQueueOutcome, RecordingQueuePolicy};
    use cockpit_simulation_core::StepRecord;

    fn step(tick: u64) -> StepRecord {
        serde_json::from_value(serde_json::json!({
            "tick": tick,
            "snapshotHash": "hash",
            "events": [],
            "observation": {
                "observationId": "observation",
                "runId": "run",
                "agentId": "agent",
                "sensorId": "sensor",
                "observedTick": tick,
                "deliveredTick": tick,
                "visibleEntities": [],
                "alerts": [],
                "actionResults": [],
                "confidence": 1.0,
                "quality": {
                    "visibilityQuality": 1.0,
                    "audioQuality": 1.0,
                    "confidence": 1.0,
                    "degraded": false
                }
            },
            "actionResults": [],
            "toolCalls": [],
            "errors": [],
            "fallback": null,
            "stateDiffs": [],
            "pluginFailures": []
        }))
        .expect("step record")
    }

    #[test]
    fn overflow_policy_is_bounded_and_observable() {
        for (policy, expected) in [
            (
                RecordingQueuePolicy::PauseRun,
                RecordingQueueOutcome::Paused,
            ),
            (RecordingQueuePolicy::FailRun, RecordingQueueOutcome::Failed),
            (RecordingQueuePolicy::Drop, RecordingQueueOutcome::Dropped),
        ] {
            let mut queue = RecordingQueue::new(1, policy);
            assert_eq!(queue.push(step(0)), RecordingQueueOutcome::Enqueued);
            assert_eq!(queue.push(step(1)), expected);
            let health = queue.health();
            assert_eq!(health.capacity, 1);
            assert_eq!(health.depth, 1);
            assert_eq!(health.enqueued, 1);
            assert_eq!(
                health.dropped,
                u64::from(expected == RecordingQueueOutcome::Dropped)
            );
            assert_eq!(
                health.rejected,
                u64::from(matches!(
                    expected,
                    RecordingQueueOutcome::Paused | RecordingQueueOutcome::Failed
                ))
            );
        }
    }
}
