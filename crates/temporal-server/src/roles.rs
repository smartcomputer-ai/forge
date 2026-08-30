//! Process roles. One binary runs any combination of roles in one process
//! (all of them by default); each worker role is its own Temporal task queue
//! with the workflow types, activities, and background loops of that
//! subsystem. `--task-types` further splits a process's worker roles into
//! workflow-only or activity-only pollers.

use std::{collections::BTreeSet, fmt, str::FromStr};

use temporalio_common::worker::WorkerTaskTypes;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// HTTP/JSON-RPC gateway, webhook hooks, environment reconciler, power
    /// reaper.
    Gateway,
    /// Session, sub-agent, and environment-job workflows with their
    /// activities and the promise reaper.
    Sessions,
    /// Bot controller and trigger-fire workflows with their activities and
    /// the schedule reconciler.
    Bots,
    /// Conversation workflows with their activities.
    Channels,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Gateway, Role::Sessions, Role::Bots, Role::Channels];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Sessions => "sessions",
            Self::Bots => "bots",
            Self::Channels => "channels",
        }
    }

    pub fn is_worker(self) -> bool {
        !matches!(self, Self::Gateway)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "gateway" => Ok(Self::Gateway),
            "sessions" => Ok(Self::Sessions),
            "bots" => Ok(Self::Bots),
            "channels" => Ok(Self::Channels),
            other => Err(format!(
                "unknown role {other:?}; expected a comma-separated subset of gateway, sessions, bots, channels"
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleSet(BTreeSet<Role>);

impl RoleSet {
    pub fn all() -> Self {
        Self(Role::ALL.into_iter().collect())
    }

    /// Parse `gateway,sessions,…`; `all` and an empty value mean every role.
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.is_empty() || value == "all" {
            return Ok(Self::all());
        }
        let mut roles = BTreeSet::new();
        for part in value.split(',') {
            roles.insert(part.parse::<Role>()?);
        }
        Ok(Self(roles))
    }

    pub fn has(&self, role: Role) -> bool {
        self.0.contains(&role)
    }

    pub fn iter(&self) -> impl Iterator<Item = Role> + '_ {
        self.0.iter().copied()
    }

    pub fn worker_roles(&self) -> impl Iterator<Item = Role> + '_ {
        self.iter().filter(|role| role.is_worker())
    }

    pub fn has_worker(&self) -> bool {
        self.worker_roles().next().is_some()
    }
}

impl fmt::Display for RoleSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.iter().map(Role::as_str).collect();
        f.write_str(&names.join(","))
    }
}

/// Which task types a process's worker roles poll.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskTypes {
    #[default]
    All,
    Workflows,
    Activities,
}

impl TaskTypes {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "" | "all" => Ok(Self::All),
            "workflows" => Ok(Self::Workflows),
            "activities" => Ok(Self::Activities),
            other => Err(format!(
                "unknown task types {other:?}; expected all, workflows, or activities"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Workflows => "workflows",
            Self::Activities => "activities",
        }
    }

    /// Local activities run inside the workflow poller, so they follow the
    /// workflow side of the split.
    pub fn worker_task_types(self) -> WorkerTaskTypes {
        match self {
            Self::All => WorkerTaskTypes::all(),
            Self::Workflows => WorkerTaskTypes {
                enable_workflows: true,
                enable_local_activities: true,
                enable_remote_activities: false,
                enable_nexus: false,
            },
            Self::Activities => WorkerTaskTypes {
                enable_workflows: false,
                enable_local_activities: false,
                enable_remote_activities: true,
                enable_nexus: false,
            },
        }
    }
}

impl fmt::Display for TaskTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_sets_parse_and_default_to_all() {
        assert_eq!(RoleSet::parse("").unwrap(), RoleSet::all());
        assert_eq!(RoleSet::parse("all").unwrap(), RoleSet::all());
        let set = RoleSet::parse("bots, gateway").unwrap();
        assert!(set.has(Role::Gateway));
        assert!(set.has(Role::Bots));
        assert!(!set.has(Role::Sessions));
        assert_eq!(set.to_string(), "gateway,bots");
        assert_eq!(set.worker_roles().collect::<Vec<_>>(), vec![Role::Bots]);
        assert!(RoleSet::parse("worker").is_err());
    }

    #[test]
    fn task_types_split_the_poller() {
        let workflows = TaskTypes::parse("workflows").unwrap().worker_task_types();
        assert!(workflows.enable_workflows && !workflows.enable_remote_activities);
        let activities = TaskTypes::parse("activities").unwrap().worker_task_types();
        assert!(!activities.enable_workflows && activities.enable_remote_activities);
        assert_eq!(TaskTypes::parse("").unwrap(), TaskTypes::All);
        assert!(TaskTypes::parse("nexus").is_err());
    }
}
