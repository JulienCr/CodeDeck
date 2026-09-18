use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessOutputEvent {
    pub(crate) run_id: String,
    pub(crate) stream: String,
    pub(crate) line: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessExitEvent {
    pub(crate) run_id: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) success: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProcessStarted {
    pub(crate) pid: u32,
}

/// The Windows PID of a WSL launcher is not ownership of the Linux process
/// tree; `pgid` starts absent and is filled in once the sentinel line arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProcessHandle {
    Native {
        pid: u32,
    },
    Wsl {
        launcher_pid: u32,
        distro: String,
        pgid: Option<i32>,
    },
}

/// Tracks the running children keyed by run id. Lookups recover from a
/// poisoned lock instead of panicking: one reader thread panicking must not
/// make Stop unusable for every other process.
#[derive(Default)]
pub(crate) struct ProcessRegistry {
    handles: Mutex<HashMap<String, ProcessHandle>>,
}

impl ProcessRegistry {
    pub(crate) fn register(&self, run_id: String, handle: ProcessHandle) {
        self.lock().insert(run_id, handle);
    }

    pub(crate) fn set_pgid(&self, run_id: &str, pgid: i32) {
        if let Some(ProcessHandle::Wsl { pgid: slot, .. }) = self.lock().get_mut(run_id) {
            *slot = Some(pgid);
        }
    }

    pub(crate) fn get(&self, run_id: &str) -> Option<ProcessHandle> {
        self.lock().get(run_id).cloned()
    }

    pub(crate) fn take(&self, run_id: &str) -> Option<ProcessHandle> {
        self.lock().remove(run_id)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ProcessHandle>> {
        self.handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_set_pgid_then_read_back() {
        let registry = ProcessRegistry::default();
        registry.register(
            "run-1".to_string(),
            ProcessHandle::Wsl {
                launcher_pid: 111,
                distro: "Ubuntu".to_string(),
                pgid: None,
            },
        );
        registry.set_pgid("run-1", 222);
        assert_eq!(
            registry.get("run-1"),
            Some(ProcessHandle::Wsl {
                launcher_pid: 111,
                distro: "Ubuntu".to_string(),
                pgid: Some(222),
            })
        );
    }

    #[test]
    fn take_removes_the_entry() {
        let registry = ProcessRegistry::default();
        registry.register("run-1".to_string(), ProcessHandle::Native { pid: 42 });
        assert!(registry.take("run-1").is_some());
        assert_eq!(registry.get("run-1"), None);
    }

    #[test]
    fn set_pgid_does_not_affect_a_native_handle() {
        let registry = ProcessRegistry::default();
        registry.register("run-1".to_string(), ProcessHandle::Native { pid: 42 });
        registry.set_pgid("run-1", 999);
        assert_eq!(
            registry.get("run-1"),
            Some(ProcessHandle::Native { pid: 42 })
        );
    }

    #[test]
    fn get_on_a_missing_run_id_returns_none() {
        let registry = ProcessRegistry::default();
        assert_eq!(registry.get("missing"), None);
    }
}
