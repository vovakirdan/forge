//! Volatile owner sessions; no provider credentials or persistent secret state.

use std::{
    collections::HashMap,
    fmt,
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const CODE_TTL: Duration = Duration::from_secs(300);
const SESSION_TTL: Duration = Duration::from_secs(8 * 60 * 60);
const SESSION_LIMIT: usize = 16;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("authentication required")]
    Unauthorized,
    #[error("session capacity reached")]
    Capacity,
    #[error("authentication unavailable")]
    Unavailable,
}

pub struct IssuedCode {
    pub code: String,
    pub expires_at: String,
}

impl fmt::Debug for IssuedCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedCode")
            .field("code", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
pub struct IssuedSession {
    pub token: String,
    pub expires_at: String,
}

impl fmt::Debug for IssuedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedSession")
            .field("token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

struct PendingCode {
    digest: [u8; 32],
    expires: Instant,
    attempts_left: u8,
}

#[derive(Default)]
struct State {
    pending: Option<PendingCode>,
    sessions: HashMap<[u8; 32], Instant>,
}

/// A process-local session registry. Only the private owner control socket issues codes.
#[derive(Default)]
pub struct SessionStore(Mutex<State>);

impl SessionStore {
    pub fn issue_code(&self) -> Result<IssuedCode, AuthError> {
        self.issue_at(Instant::now(), OffsetDateTime::now_utc())
    }

    fn issue_at(&self, now: Instant, wall: OffsetDateTime) -> Result<IssuedCode, AuthError> {
        let code = random_secret()?;
        let expires_at = display_expiry(wall, CODE_TTL)?;
        self.0.lock().map_err(|_| AuthError::Unavailable)?.pending = Some(PendingCode {
            digest: digest(&code),
            expires: now + CODE_TTL,
            attempts_left: 5,
        });
        Ok(IssuedCode { code, expires_at })
    }

    pub fn exchange(&self, code: &str) -> Result<IssuedSession, AuthError> {
        self.exchange_at(code, Instant::now(), OffsetDateTime::now_utc())
    }

    fn exchange_at(
        &self,
        code: &str,
        now: Instant,
        wall: OffsetDateTime,
    ) -> Result<IssuedSession, AuthError> {
        let mut state = self.0.lock().map_err(|_| AuthError::Unavailable)?;
        state.sessions.retain(|_, expires| *expires > now);
        let Some(pending) = &mut state.pending else {
            return Err(AuthError::Unauthorized);
        };
        if pending.expires <= now {
            state.pending = None;
            return Err(AuthError::Unauthorized);
        }
        if !bool::from(pending.digest.ct_eq(&digest(code))) {
            pending.attempts_left -= 1;
            if pending.attempts_left == 0 {
                state.pending = None;
            }
            return Err(AuthError::Unauthorized);
        }
        if state.sessions.len() >= SESSION_LIMIT {
            return Err(AuthError::Capacity);
        }
        let token = random_secret()?;
        let expires_at = display_expiry(wall, SESSION_TTL)?;
        state.pending = None;
        state.sessions.insert(digest(&token), now + SESSION_TTL);
        Ok(IssuedSession { token, expires_at })
    }

    pub fn authorize(&self, token: &str) -> Result<(), AuthError> {
        self.authorize_at(token, Instant::now())
    }

    fn authorize_at(&self, token: &str, now: Instant) -> Result<(), AuthError> {
        let mut state = self.0.lock().map_err(|_| AuthError::Unavailable)?;
        state.sessions.retain(|_, expires| *expires > now);
        if state.sessions.contains_key(&digest(token)) {
            Ok(())
        } else {
            Err(AuthError::Unauthorized)
        }
    }

    pub fn logout(&self, token: &str) -> Result<(), AuthError> {
        self.0
            .lock()
            .map_err(|_| AuthError::Unavailable)?
            .sessions
            .remove(&digest(token));
        Ok(())
    }

    pub fn revoke_all(&self) {
        if let Ok(mut state) = self.0.lock() {
            *state = State::default();
        }
    }
}

fn random_secret() -> Result<String, AuthError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::Unavailable)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn display_expiry(now: OffsetDateTime, ttl: Duration) -> Result<String, AuthError> {
    (now + ttl)
        .format(&Rfc3339)
        .map_err(|_| AuthError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_uses_monotonic_time_and_reissue_invalidates_previous_code() {
        let store = SessionStore::default();
        let now = Instant::now();
        let wall = OffsetDateTime::UNIX_EPOCH;
        let first = store.issue_at(now, wall).unwrap();
        let second = store.issue_at(now, wall).unwrap();
        assert!(matches!(
            store.exchange_at(&first.code, now, wall),
            Err(AuthError::Unauthorized)
        ));
        assert!(matches!(
            store.exchange_at(&second.code, now + CODE_TTL, wall),
            Err(AuthError::Unauthorized)
        ));
    }

    #[test]
    fn five_failed_attempts_invalidate_code() {
        let store = SessionStore::default();
        let code = store.issue_code().unwrap();
        for _ in 0..5 {
            assert!(matches!(
                store.exchange("wrong"),
                Err(AuthError::Unauthorized)
            ));
        }
        assert!(matches!(
            store.exchange(&code.code),
            Err(AuthError::Unauthorized)
        ));
    }

    #[test]
    fn session_capacity_logout_expiry_and_restart_are_fail_closed() {
        let store = SessionStore::default();
        let now = Instant::now();
        let wall = OffsetDateTime::UNIX_EPOCH;
        let mut tokens = Vec::new();
        for _ in 0..SESSION_LIMIT {
            let code = store.issue_at(now, wall).unwrap();
            tokens.push(store.exchange_at(&code.code, now, wall).unwrap().token);
        }
        let code = store.issue_at(now, wall).unwrap();
        assert!(matches!(
            store.exchange_at(&code.code, now, wall),
            Err(AuthError::Capacity)
        ));
        store.logout(&tokens[0]).unwrap();
        assert!(store.authorize_at(&tokens[0], now).is_err());
        let session = store.exchange_at(&code.code, now, wall).unwrap();
        assert!(
            store
                .authorize_at(&session.token, now + SESSION_TTL)
                .is_err()
        );
        assert!(SessionStore::default().authorize(&tokens[1]).is_err());
    }

    #[test]
    fn exchange_race_has_one_winner_and_debug_redacts_secrets() {
        let store = std::sync::Arc::new(SessionStore::default());
        let code = store.issue_code().unwrap();
        assert!(!format!("{code:?}").contains(&code.code));
        let results = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| store.exchange(&code.code)))
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let session = results.into_iter().find_map(Result::ok).unwrap();
        assert!(!format!("{session:?}").contains(&session.token));
        store.revoke_all();
        assert!(store.authorize(&session.token).is_err());
    }
}
