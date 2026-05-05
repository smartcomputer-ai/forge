//! Session, run, and turn lifecycle records.
//!
//! This module will contain lifecycle/status enums and transition validation
//! helpers.

use crate::error::ModelError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    New,
    Active,
    Paused,
    Closing,
    Closed,
}

impl SessionStatus {
    pub fn accepts_new_runs(self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Closed)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::New, Self::Active)
                | (Self::New, Self::Closed)
                | (Self::Active, Self::Paused)
                | (Self::Active, Self::Closing)
                | (Self::Active, Self::Closed)
                | (Self::Paused, Self::Active)
                | (Self::Paused, Self::Closing)
                | (Self::Paused, Self::Closed)
                | (Self::Closing, Self::Closed)
        )
    }

    pub fn transition_to(self, next: Self) -> Result<Self, ModelError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(ModelError::InvalidLifecycleTransition {
                kind: "session_status",
                from: format!("{self:?}"),
                to: format!("{next:?}"),
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLifecycle {
    #[default]
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl RunLifecycle {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Queued, Self::Running)
                | (Self::Queued, Self::Cancelled)
                | (Self::Running, Self::Waiting)
                | (Self::Waiting, Self::Running)
                | (Self::Running, Self::Completed)
                | (Self::Waiting, Self::Completed)
                | (Self::Running, Self::Failed)
                | (Self::Waiting, Self::Failed)
                | (Self::Running, Self::Cancelled)
                | (Self::Waiting, Self::Cancelled)
                | (Self::Running, Self::Interrupted)
                | (Self::Waiting, Self::Interrupted)
        )
    }

    pub fn transition_to(self, next: Self) -> Result<Self, ModelError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(ModelError::InvalidLifecycleTransition {
                kind: "run_lifecycle",
                from: format!("{self:?}"),
                to: format!("{next:?}"),
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnLifecycle {
    #[default]
    Planned,
    Requesting,
    WaitingForTools,
    Completed,
    Failed,
    Cancelled,
}

impl TurnLifecycle {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Planned, Self::Requesting)
                | (Self::Planned, Self::Cancelled)
                | (Self::Requesting, Self::WaitingForTools)
                | (Self::Requesting, Self::Completed)
                | (Self::Requesting, Self::Failed)
                | (Self::Requesting, Self::Cancelled)
                | (Self::WaitingForTools, Self::Requesting)
                | (Self::WaitingForTools, Self::Completed)
                | (Self::WaitingForTools, Self::Failed)
                | (Self::WaitingForTools, Self::Cancelled)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_status_rejects_reopening_closed_session() {
        let error = SessionStatus::Closed
            .transition_to(SessionStatus::Active)
            .expect_err("closed sessions cannot reopen");
        assert!(matches!(
            error,
            ModelError::InvalidLifecycleTransition {
                kind: "session_status",
                ..
            }
        ));
    }

    #[test]
    fn run_lifecycle_allows_waiting_resume_and_terminal_completion() {
        assert_eq!(
            RunLifecycle::Running.transition_to(RunLifecycle::Waiting),
            Ok(RunLifecycle::Waiting)
        );
        assert_eq!(
            RunLifecycle::Waiting.transition_to(RunLifecycle::Completed),
            Ok(RunLifecycle::Completed)
        );
        assert!(RunLifecycle::Completed.is_terminal());
    }
}
