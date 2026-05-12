//! Core types for graph analysis.
//!
//! These types represent issues, dependencies, and their statuses for use
//! by the native graph engine (ported from beads_viewer).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Issue status in the workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Open,
    InProgress,
    Blocked,
    Deferred,
    Closed,
    Tombstone,
    Pinned,
    Hooked,
}

impl IssueStatus {
    /// Check if the status represents an active (not terminal) state.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Open | Self::InProgress | Self::Blocked | Self::Pinned | Self::Hooked
        )
    }

    /// Check if the status represents a terminal state.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Tombstone)
    }
}

impl std::fmt::Display for IssueStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Deferred => "deferred",
            Self::Closed => "closed",
            Self::Tombstone => "tombstone",
            Self::Pinned => "pinned",
            Self::Hooked => "hooked",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for IssueStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "open" => Ok(Self::Open),
            "in_progress" | "in-progress" | "inprogress" => Ok(Self::InProgress),
            "blocked" => Ok(Self::Blocked),
            "deferred" => Ok(Self::Deferred),
            "closed" => Ok(Self::Closed),
            "tombstone" => Ok(Self::Tombstone),
            "pinned" => Ok(Self::Pinned),
            "hooked" => Ok(Self::Hooked),
            _ => Err(format!("unknown issue status: {s}")),
        }
    }
}

/// Issue type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    #[default]
    Task,
    Bug,
    Feature,
    Epic,
    Chore,
    Message,
    Gate,
    Agent,
    Role,
    Convoy,
    Event,
    Slot,
    Question,
    Docs,
}

impl std::fmt::Display for IssueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Task => "task",
            Self::Bug => "bug",
            Self::Feature => "feature",
            Self::Epic => "epic",
            Self::Chore => "chore",
            Self::Message => "message",
            Self::Gate => "gate",
            Self::Agent => "agent",
            Self::Role => "role",
            Self::Convoy => "convoy",
            Self::Event => "event",
            Self::Slot => "slot",
            Self::Question => "question",
            Self::Docs => "docs",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for IssueType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "task" => Ok(Self::Task),
            "bug" => Ok(Self::Bug),
            "feature" => Ok(Self::Feature),
            "epic" => Ok(Self::Epic),
            "chore" => Ok(Self::Chore),
            "message" => Ok(Self::Message),
            "gate" => Ok(Self::Gate),
            "agent" => Ok(Self::Agent),
            "role" => Ok(Self::Role),
            "convoy" => Ok(Self::Convoy),
            "event" => Ok(Self::Event),
            "slot" => Ok(Self::Slot),
            "question" => Ok(Self::Question),
            "docs" => Ok(Self::Docs),
            _ => Err(format!("unknown issue type: {s}")),
        }
    }
}

/// Priority level (0 = critical, 4 = backlog).
pub type Priority = u8;

/// A dependency relationship between issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// The related issue ID
    pub id: String,

    /// Title of the related issue
    #[serde(default)]
    pub title: String,

    /// Status of the related issue
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<IssueStatus>,

    /// Type of dependency relationship
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_type: Option<DependencyType>,
}

/// Types of dependency relationships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
    /// This issue blocks another
    Blocks,
    /// This issue is blocked by another
    BlockedBy,
    /// Parent-child relationship
    Parent,
    Child,
    /// Conditional blocking
    ConditionalBlocks,
    /// Waiting for completion
    WaitsFor,
    /// Tracking relationship
    Tracks,
}

/// An issue (node in the dependency graph).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// Unique issue ID
    pub id: String,

    /// Issue title
    pub title: String,

    /// Issue description (Markdown)
    #[serde(default)]
    pub description: String,

    /// Current status
    pub status: IssueStatus,

    /// Priority (0-4, lower = higher priority)
    #[serde(default)]
    pub priority: Priority,

    /// Issue type classification
    #[serde(default)]
    pub issue_type: IssueType,

    /// Assigned owner (email or username)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,

    /// Assigned worker
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,

    /// Labels/tags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,

    /// Notes (additional context)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    /// Creation timestamp
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,

    /// Creator identity
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,

    /// Last update timestamp
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,

    /// Closed timestamp (if closed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,

    /// Issues that this issue depends on (blockers)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<Dependency>,

    /// Issues that depend on this issue (dependents)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependents: Vec<Dependency>,

    /// Unknown fields captured for forward compatibility
    #[serde(default, skip_serializing_if = "HashMap::is_empty", flatten)]
    pub extra: HashMap<String, JsonValue>,
}

impl Issue {
    /// Check if this issue is ready to work (open and not blocked).
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.status == IssueStatus::Open && self.dependencies.is_empty()
    }

    /// Check if this issue is in an active (workable) state.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_status_roundtrip() {
        let statuses = [
            IssueStatus::Open,
            IssueStatus::InProgress,
            IssueStatus::Blocked,
            IssueStatus::Deferred,
            IssueStatus::Closed,
            IssueStatus::Tombstone,
            IssueStatus::Pinned,
            IssueStatus::Hooked,
        ];

        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: IssueStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn test_issue_type_roundtrip() {
        let types = [
            IssueType::Task,
            IssueType::Bug,
            IssueType::Feature,
            IssueType::Epic,
            IssueType::Chore,
            IssueType::Question,
            IssueType::Docs,
        ];

        for issue_type in types {
            let json = serde_json::to_string(&issue_type).unwrap();
            let parsed: IssueType = serde_json::from_str(&json).unwrap();
            assert_eq!(issue_type, parsed);
        }
    }

    #[test]
    fn test_issue_status_from_str() {
        assert_eq!("open".parse::<IssueStatus>().unwrap(), IssueStatus::Open);
        assert_eq!(
            "in_progress".parse::<IssueStatus>().unwrap(),
            IssueStatus::InProgress
        );
        assert_eq!(
            "closed".parse::<IssueStatus>().unwrap(),
            IssueStatus::Closed
        );
    }

    #[test]
    fn test_issue_status_display() {
        assert_eq!(IssueStatus::Open.to_string(), "open");
        assert_eq!(IssueStatus::InProgress.to_string(), "in_progress");
        assert_eq!(IssueStatus::Blocked.to_string(), "blocked");
    }

    #[test]
    fn test_issue_is_ready() {
        let ready_issue = Issue {
            id: "test-1".to_string(),
            title: "Ready".to_string(),
            description: String::new(),
            status: IssueStatus::Open,
            priority: 0,
            issue_type: IssueType::Task,
            owner: None,
            assignee: None,
            labels: vec![],
            notes: None,
            created_at: None,
            created_by: None,
            updated_at: None,
            closed_at: None,
            dependencies: vec![],
            dependents: vec![],
            extra: HashMap::new(),
        };
        assert!(ready_issue.is_ready());
    }

    #[test]
    fn test_issue_not_ready() {
        let blocked_issue = Issue {
            id: "test-2".to_string(),
            title: "Blocked".to_string(),
            description: String::new(),
            status: IssueStatus::Open,
            priority: 0,
            issue_type: IssueType::Task,
            owner: None,
            assignee: None,
            labels: vec![],
            notes: None,
            created_at: None,
            created_by: None,
            updated_at: None,
            closed_at: None,
            dependencies: vec![Dependency {
                id: "blocker-1".to_string(),
                title: "Blocker".to_string(),
                status: None,
                dependency_type: None,
            }],
            dependents: vec![],
            extra: HashMap::new(),
        };
        assert!(!blocked_issue.is_ready());
    }
}
