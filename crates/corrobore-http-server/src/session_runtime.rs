// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{ActorId, SessionId, WorkspaceId};
use serde::{Deserialize, Serialize};
use shared_runtime::{
    ActorKind, ActorRef, RuntimeError, RuntimeSessionMetadata, RuntimeTimestamp, SessionRegistry,
    StartSessionRequest,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionServiceStatus {
    Idle,
    Working,
    Processing,
    Degraded,
    Stopped,
}

impl SessionServiceStatus {
    pub fn ensure_valid_transition(
        self,
        next: SessionServiceStatus,
    ) -> Result<(), SessionRuntimeError> {
        if is_transition_allowed(self, next) {
            return Ok(());
        }

        Err(SessionRuntimeError::InvalidStatusTransition {
            from: self,
            to: next,
        })
    }
}

fn is_transition_allowed(from: SessionServiceStatus, to: SessionServiceStatus) -> bool {
    use SessionServiceStatus::{Degraded, Idle, Processing, Stopped, Working};

    match from {
        Idle => matches!(to, Working | Processing | Degraded | Stopped),
        Working => matches!(to, Processing | Idle | Degraded | Stopped),
        Processing => matches!(to, Working | Idle | Degraded | Stopped),
        Degraded => matches!(to, Idle | Stopped),
        Stopped => matches!(to, Stopped),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHealthView {
    pub session_id: String,
    pub workspace_id: String,
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub status: SessionServiceStatus,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub auto_stopped_due_to_idle_ttl: bool,
}

#[derive(Clone, Debug)]
pub struct StartSessionInput {
    pub workspace_id: String,
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct StartedSession {
    pub session_id: String,
    pub status: SessionServiceStatus,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionExpirationMetrics {
    pub total_expired_sessions: u64,
    pub expired_last_5m_sessions: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedSessionState {
    sessions: Vec<PersistedSessionRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedSessionRecord {
    session_id: String,
    workspace_id: String,
    actor_id: String,
    actor_kind: ActorKind,
    status: SessionServiceStatus,
    started_at_ms: u64,
    updated_at_ms: u64,
    #[serde(default)]
    auto_stopped_due_to_idle_ttl: bool,
    metadata: HashMap<String, String>,
}

pub struct SessionRuntime {
    registry: SessionRegistry,
    sessions: HashMap<String, PersistedSessionRecord>,
    store_file: PathBuf,
    session_idle_ttl_ms: u64,
    total_expired_sessions: u64,
    expiration_events_ms: VecDeque<u64>,
}

impl SessionRuntime {
    pub fn new(store_dir: impl Into<String>, session_idle_ttl_ms: u64) -> Self {
        let store_file = Path::new(&store_dir.into()).join("sessions.json");

        let mut runtime = Self {
            registry: SessionRegistry::default(),
            sessions: HashMap::new(),
            store_file,
            session_idle_ttl_ms,
            total_expired_sessions: 0,
            expiration_events_ms: VecDeque::new(),
        };

        let _ = runtime.load_from_disk();

        runtime
    }

    pub fn expire_inactive_sessions(&mut self) -> Result<usize, SessionRuntimeError> {
        if self.session_idle_ttl_ms == 0 {
            return Ok(0);
        }

        let now = current_millis()?;
        let mut expired_count = 0usize;

        for entry in self.sessions.values_mut() {
            if entry.status == SessionServiceStatus::Stopped {
                continue;
            }

            let inactive_for_ms = now.saturating_sub(entry.updated_at_ms);
            if inactive_for_ms >= self.session_idle_ttl_ms {
                entry.status = SessionServiceStatus::Stopped;
                entry.updated_at_ms = now;
                entry.auto_stopped_due_to_idle_ttl = true;
                expired_count += 1;
            }
        }

        if expired_count > 0 {
            let expired_count_u64 = u64::try_from(expired_count)
                .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;
            self.total_expired_sessions = self
                .total_expired_sessions
                .saturating_add(expired_count_u64);
            for _ in 0..expired_count {
                self.expiration_events_ms.push_back(now);
            }
            self.prune_expiration_events(now);
            self.persist_to_disk()?;
        }

        Ok(expired_count)
    }

    pub fn start_session(
        &mut self,
        input: StartSessionInput,
    ) -> Result<StartedSession, SessionRuntimeError> {
        let session_uuid = Uuid::new_v4().to_string();
        let session_id = SessionId::new(session_uuid.clone())
            .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;
        let workspace_id = WorkspaceId::new(input.workspace_id.clone())
            .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;
        let actor_id = ActorId::new(input.actor_id.clone())
            .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;
        let now = current_millis()?;

        self.registry
            .start_session(StartSessionRequest {
                id: session_id,
                actor: Some(ActorRef::new(actor_id, input.actor_kind.clone())),
                workspace_id,
                started_at: RuntimeTimestamp::from_millis(now),
                metadata: input.metadata.clone(),
            })
            .map_err(map_runtime_error)?;

        let persisted = PersistedSessionRecord {
            session_id: session_uuid.clone(),
            workspace_id: input.workspace_id,
            actor_id: input.actor_id,
            actor_kind: input.actor_kind,
            status: SessionServiceStatus::Idle,
            started_at_ms: now,
            updated_at_ms: now,
            auto_stopped_due_to_idle_ttl: false,
            metadata: input.metadata,
        };

        self.sessions.insert(session_uuid.clone(), persisted);
        self.persist_to_disk()?;

        Ok(StartedSession {
            session_id: session_uuid,
            status: SessionServiceStatus::Idle,
        })
    }

    pub fn session_health(
        &self,
        session_id: &str,
    ) -> Result<SessionHealthView, SessionRuntimeError> {
        let entry = self
            .sessions
            .get(session_id)
            .ok_or_else(|| SessionRuntimeError::SessionNotFound(session_id.to_owned()))?;

        Ok(SessionHealthView {
            session_id: entry.session_id.clone(),
            workspace_id: entry.workspace_id.clone(),
            actor_id: entry.actor_id.clone(),
            actor_kind: entry.actor_kind.clone(),
            status: entry.status,
            started_at_ms: entry.started_at_ms,
            updated_at_ms: entry.updated_at_ms,
            auto_stopped_due_to_idle_ttl: entry.auto_stopped_due_to_idle_ttl,
        })
    }

    /// List session read models in stable identifier order.
    ///
    /// Phase three will expire inactive sessions before the handler calls this
    /// method and filter stopped records unless explicitly requested.
    pub fn list_sessions(&self, include_stopped: bool) -> Vec<SessionHealthView> {
        let mut sessions = self
            .sessions
            .values()
            .filter(|entry| include_stopped || entry.status != SessionServiceStatus::Stopped)
            .map(|entry| SessionHealthView {
                session_id: entry.session_id.clone(),
                workspace_id: entry.workspace_id.clone(),
                actor_id: entry.actor_id.clone(),
                actor_kind: entry.actor_kind.clone(),
                status: entry.status,
                started_at_ms: entry.started_at_ms,
                updated_at_ms: entry.updated_at_ms,
                auto_stopped_due_to_idle_ttl: entry.auto_stopped_due_to_idle_ttl,
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        sessions
    }

    pub fn transition_to(
        &mut self,
        session_id: &str,
        next: SessionServiceStatus,
    ) -> Result<(), SessionRuntimeError> {
        let now = current_millis()?;
        let entry = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| SessionRuntimeError::SessionNotFound(session_id.to_owned()))?;

        entry.status.ensure_valid_transition(next)?;
        entry.status = next;
        entry.updated_at_ms = now;
        entry.auto_stopped_due_to_idle_ttl = false;

        self.persist_to_disk()
    }

    pub fn stop_session(&mut self, session_id: &str) -> Result<(), SessionRuntimeError> {
        self.transition_to(session_id, SessionServiceStatus::Stopped)
    }

    fn load_from_disk(&mut self) -> Result<(), SessionRuntimeError> {
        if !self.store_file.exists() {
            return Ok(());
        }

        let bytes = fs::read(&self.store_file)
            .map_err(|error| SessionRuntimeError::Persistence(error.to_string()))?;
        let persisted: PersistedSessionState = serde_json::from_slice(&bytes)
            .map_err(|error| SessionRuntimeError::Persistence(error.to_string()))?;

        for entry in persisted.sessions {
            let session_id = SessionId::new(entry.session_id.clone())
                .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;
            let workspace_id = WorkspaceId::new(entry.workspace_id.clone())
                .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;
            let actor_id = ActorId::new(entry.actor_id.clone())
                .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;

            self.registry
                .start_session(StartSessionRequest {
                    id: session_id,
                    actor: Some(ActorRef::new(actor_id, entry.actor_kind.clone())),
                    workspace_id,
                    started_at: RuntimeTimestamp::from_millis(entry.started_at_ms),
                    metadata: entry.metadata.clone(),
                })
                .map_err(map_runtime_error)?;

            self.sessions.insert(entry.session_id.clone(), entry);
        }

        Ok(())
    }

    fn persist_to_disk(&self) -> Result<(), SessionRuntimeError> {
        if let Some(parent) = self.store_file.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| SessionRuntimeError::Persistence(error.to_string()))?;
        }

        let state = PersistedSessionState {
            sessions: self.sessions.values().cloned().collect(),
        };

        let payload = serde_json::to_vec_pretty(&state)
            .map_err(|error| SessionRuntimeError::Persistence(error.to_string()))?;
        fs::write(&self.store_file, payload)
            .map_err(|error| SessionRuntimeError::Persistence(error.to_string()))
    }

    pub fn map_runtime_metadata(
        &self,
        session_id: &str,
    ) -> Result<RuntimeSessionMetadata, SessionRuntimeError> {
        let typed = SessionId::new(session_id.to_owned())
            .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;

        self.registry
            .read_session_metadata(&typed)
            .map_err(map_runtime_error)
    }

    pub fn expiration_metrics(&self) -> Result<SessionExpirationMetrics, SessionRuntimeError> {
        let now = current_millis()?;
        let cutoff = now.saturating_sub(5 * 60 * 1000);
        let expired_last_5m_sessions = self
            .expiration_events_ms
            .iter()
            .filter(|timestamp_ms| **timestamp_ms >= cutoff)
            .count();
        let expired_last_5m_sessions = u64::try_from(expired_last_5m_sessions)
            .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;

        Ok(SessionExpirationMetrics {
            total_expired_sessions: self.total_expired_sessions,
            expired_last_5m_sessions,
        })
    }

    fn prune_expiration_events(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(5 * 60 * 1000);
        while let Some(oldest) = self.expiration_events_ms.front() {
            if *oldest >= cutoff {
                break;
            }
            let _ = self.expiration_events_ms.pop_front();
        }
    }
}

#[derive(Debug)]
pub enum SessionRuntimeError {
    SessionNotFound(String),
    InvalidStatusTransition {
        from: SessionServiceStatus,
        to: SessionServiceStatus,
    },
    Persistence(String),
    Runtime(String),
}

fn map_runtime_error(error: RuntimeError) -> SessionRuntimeError {
    match error {
        RuntimeError::SessionNotFound(session_id) => {
            SessionRuntimeError::SessionNotFound(session_id.as_str().to_owned())
        }
        other => SessionRuntimeError::Runtime(other.to_string()),
    }
}

fn current_millis() -> Result<u64, SessionRuntimeError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;

    let millis = u64::try_from(duration.as_millis())
        .map_err(|error| SessionRuntimeError::Runtime(error.to_string()))?;

    Ok(millis)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{SessionRuntime, SessionServiceStatus, StartSessionInput};

    fn unique_store_dir(suffix: &str) -> String {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_millis();

        let path: PathBuf = std::env::temp_dir().join(format!(
            "corrobore-session-runtime-tests-{}-{}",
            suffix, millis
        ));
        path.display().to_string()
    }

    #[test]
    fn session_status_fsm_rejects_invalid_transition() {
        let error = SessionServiceStatus::Idle
            .ensure_valid_transition(SessionServiceStatus::Stopped)
            .err();

        assert!(error.is_none(), "idle to stopped should be allowed");

        let invalid = SessionServiceStatus::Stopped
            .ensure_valid_transition(SessionServiceStatus::Working)
            .expect_err("stopped to working should be rejected");

        assert!(matches!(
            invalid,
            super::SessionRuntimeError::InvalidStatusTransition { .. }
        ));
    }

    #[test]
    fn session_runtime_persists_and_reloads_sessions() {
        let store_dir = unique_store_dir("reload");
        let mut runtime = SessionRuntime::new(store_dir.clone(), 0);

        let started = runtime
            .start_session(StartSessionInput {
                workspace_id: "workspace--session-runtime-tests".to_owned(),
                actor_id: "actor--session-runtime-tests".to_owned(),
                actor_kind: shared_runtime::ActorKind::Agent,
                metadata: Default::default(),
            })
            .expect("session should start");

        runtime
            .transition_to(&started.session_id, SessionServiceStatus::Working)
            .expect("transition to working should succeed");

        let reloaded = SessionRuntime::new(store_dir, 0);
        let health = reloaded
            .session_health(&started.session_id)
            .expect("session should be reloaded from disk");

        assert_eq!(health.status, SessionServiceStatus::Working);
    }

    #[test]
    fn session_runtime_expires_inactive_sessions_when_ttl_is_reached() {
        let store_dir = unique_store_dir("expire-idle");
        let mut runtime = SessionRuntime::new(store_dir, 1);

        let started = runtime
            .start_session(StartSessionInput {
                workspace_id: "workspace--session-runtime-expire".to_owned(),
                actor_id: "actor--session-runtime-expire".to_owned(),
                actor_kind: shared_runtime::ActorKind::Agent,
                metadata: Default::default(),
            })
            .expect("session should start");

        std::thread::sleep(std::time::Duration::from_millis(3));
        let expired = runtime
            .expire_inactive_sessions()
            .expect("expiration should succeed");

        assert_eq!(expired, 1);
        let health = runtime
            .session_health(&started.session_id)
            .expect("session should exist");
        assert_eq!(health.status, SessionServiceStatus::Stopped);
    }
}
