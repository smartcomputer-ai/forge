//! Durable agent identifiers and allocation helpers.
//!
//! This module will contain explicit id newtypes for agents, sessions, runs,
//! turns, submissions, effects, tool batches, tool calls, journal events, and
//! projection items.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(
            Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(AgentId);
string_id!(AgentVersionId);
string_id!(SessionId);
string_id!(SubmissionId);
string_id!(CorrelationId);
string_id!(ToolCallId);

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct JournalSeq(pub u64);

impl fmt::Display for JournalSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RunId {
    pub session_id: SessionId,
    pub run_seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TurnId {
    pub run_id: RunId,
    pub turn_seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolBatchId {
    pub run_id: RunId,
    pub batch_seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EffectId {
    pub session_id: SessionId,
    pub effect_seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProjectionItemId {
    pub session_id: SessionId,
    pub item_seq: u64,
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:run:{}", self.session_id, self.run_seq)
    }
}

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:turn:{}", self.run_id, self.turn_seq)
    }
}

impl fmt::Display for ToolBatchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:tool_batch:{}", self.run_id, self.batch_seq)
    }
}

impl fmt::Display for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:effect:{}", self.session_id, self.effect_seq)
    }
}

impl fmt::Display for ProjectionItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:item:{}", self.session_id, self.item_seq)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAllocator {
    pub session_id: SessionId,
    pub next_run_seq: u64,
    pub next_turn_seq: u64,
    pub next_tool_batch_seq: u64,
    pub next_effect_seq: u64,
    pub next_journal_seq: u64,
    pub next_projection_item_seq: u64,
}

impl IdAllocator {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            next_run_seq: 1,
            next_turn_seq: 1,
            next_tool_batch_seq: 1,
            next_effect_seq: 1,
            next_journal_seq: 1,
            next_projection_item_seq: 1,
        }
    }

    pub fn allocate_run_id(&mut self) -> RunId {
        let run_seq = self.next_run_seq;
        self.next_run_seq = self.next_run_seq.saturating_add(1);
        RunId {
            session_id: self.session_id.clone(),
            run_seq,
        }
    }

    pub fn allocate_turn_id(&mut self, run_id: &RunId) -> TurnId {
        let turn_seq = self.next_turn_seq;
        self.next_turn_seq = self.next_turn_seq.saturating_add(1);
        TurnId {
            run_id: run_id.clone(),
            turn_seq,
        }
    }

    pub fn allocate_tool_batch_id(&mut self, run_id: &RunId) -> ToolBatchId {
        let batch_seq = self.next_tool_batch_seq;
        self.next_tool_batch_seq = self.next_tool_batch_seq.saturating_add(1);
        ToolBatchId {
            run_id: run_id.clone(),
            batch_seq,
        }
    }

    pub fn allocate_effect_id(&mut self) -> EffectId {
        let effect_seq = self.next_effect_seq;
        self.next_effect_seq = self.next_effect_seq.saturating_add(1);
        EffectId {
            session_id: self.session_id.clone(),
            effect_seq,
        }
    }

    pub fn allocate_journal_seq(&mut self) -> JournalSeq {
        let seq = self.next_journal_seq;
        self.next_journal_seq = self.next_journal_seq.saturating_add(1);
        JournalSeq(seq)
    }

    pub fn allocate_projection_item_id(&mut self) -> ProjectionItemId {
        let item_seq = self.next_projection_item_seq;
        self.next_projection_item_seq = self.next_projection_item_seq.saturating_add(1);
        ProjectionItemId {
            session_id: self.session_id.clone(),
            item_seq,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_allocator_allocates_stable_sequences() {
        let mut allocator = IdAllocator::new(SessionId::new("session-a"));
        let run = allocator.allocate_run_id();
        let turn = allocator.allocate_turn_id(&run);
        let batch = allocator.allocate_tool_batch_id(&run);
        let effect = allocator.allocate_effect_id();
        let journal_seq = allocator.allocate_journal_seq();
        let item = allocator.allocate_projection_item_id();

        assert_eq!(run.to_string(), "session-a:run:1");
        assert_eq!(turn.to_string(), "session-a:run:1:turn:1");
        assert_eq!(batch.to_string(), "session-a:run:1:tool_batch:1");
        assert_eq!(effect.to_string(), "session-a:effect:1");
        assert_eq!(journal_seq.to_string(), "1");
        assert_eq!(item.to_string(), "session-a:item:1");
        assert_eq!(allocator.allocate_run_id().run_seq, 2);
    }

    #[test]
    fn string_ids_round_trip_through_json() {
        let submission = SubmissionId::new("submit-1");
        let encoded = serde_json::to_string(&submission).expect("serialize id");
        assert_eq!(encoded, "\"submit-1\"");
        let decoded: SubmissionId = serde_json::from_str(&encoded).expect("deserialize id");
        assert_eq!(decoded, submission);
    }
}
