use std::sync::RwLock;

use crate::types::{LoginInfo, Status};

pub struct Runtime {
    inner: RwLock<RuntimeInner>,
}

#[derive(Default)]
struct RuntimeInner {
    login: LoginInfo,
    upstream_connected: bool,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(RuntimeInner::default()),
        }
    }

    pub fn set_login(&self, info: LoginInfo) {
        let mut guard = self.inner.write().expect("Runtime rwlock poisoned");
        guard.login = info;
    }

    pub fn login(&self) -> LoginInfo {
        let guard = self.inner.read().expect("Runtime rwlock poisoned");
        guard.login.clone()
    }

    pub fn set_upstream_connected(&self, connected: bool) {
        let mut guard = self.inner.write().expect("Runtime rwlock poisoned");
        guard.upstream_connected = connected;
    }

    pub fn status(&self) -> Status {
        let guard = self.inner.read().expect("Runtime rwlock poisoned");
        Status {
            online: guard.upstream_connected,
            good: guard.upstream_connected && guard.login.self_id != 0,
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_runtime_is_offline_and_not_good() {
        let rt = Runtime::new();
        let s = rt.status();
        assert!(!s.online);
        assert!(!s.good);
    }

    #[test]
    fn upstream_connected_makes_online_but_not_good_until_login() {
        let rt = Runtime::new();
        rt.set_upstream_connected(true);
        let s = rt.status();
        assert!(s.online);
        assert!(!s.good, "good requires non-zero self_id");
    }

    #[test]
    fn login_plus_connected_is_good() {
        let rt = Runtime::new();
        rt.set_upstream_connected(true);
        rt.set_login(LoginInfo {
            self_id: 42,
            nickname: "bot".to_string(),
        });
        let s = rt.status();
        assert!(s.online);
        assert!(s.good);
    }

    #[test]
    fn login_alone_is_not_good_without_connection() {
        let rt = Runtime::new();
        rt.set_login(LoginInfo {
            self_id: 42,
            nickname: "bot".to_string(),
        });
        assert!(!rt.status().good);
    }

    #[test]
    fn login_getter_returns_clone_with_fields() {
        let rt = Runtime::new();
        rt.set_login(LoginInfo {
            self_id: 7,
            nickname: "n".to_string(),
        });
        let got = rt.login();
        assert_eq!(got.self_id, 7);
        assert_eq!(got.nickname, "n");
    }
}
