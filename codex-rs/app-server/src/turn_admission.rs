//! Tracks client work admitted before a shutdown drain.

use codex_app_server_protocol::JSONRPCErrorError;
use codex_extension_api::CoreWorkAdmission;
use codex_extension_api::CoreWorkKind;
use codex_extension_api::TurnStartAdmission;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::watch;

use crate::error_code::server_draining_error;

#[derive(Debug, Default)]
struct AdmissionState {
    closed: bool,
    active: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TurnAdmission {
    state: Arc<Mutex<AdmissionState>>,
    active_tx: watch::Sender<usize>,
    open_tx: watch::Sender<bool>,
}

impl Default for TurnAdmission {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(AdmissionState::default())),
            active_tx: watch::channel(0).0,
            open_tx: watch::channel(true).0,
        }
    }
}

pub(crate) struct TurnPermit(TurnAdmission);

impl TurnAdmission {
    pub(crate) fn begin_drain(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = true;
        self.open_tx.send_replace(false);
    }

    pub(crate) fn reopen(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = false;
        self.open_tx.send_replace(true);
    }

    pub(crate) fn subscribe_active(&self) -> watch::Receiver<usize> {
        self.active_tx.subscribe()
    }

    // Admit and close take the same short lock. The permit keeps shutdown from
    // finishing while an earlier request is still preparing or submitting work.
    pub(crate) fn admit(&self) -> Result<TurnPermit, JSONRPCErrorError> {
        self.try_admit().ok_or_else(server_draining_error)
    }

    fn try_admit(&self) -> Option<TurnPermit> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return None;
        }
        state.active += 1;
        self.active_tx.send_replace(state.active);
        Some(TurnPermit(self.clone()))
    }
}

impl TurnStartAdmission for TurnAdmission {
    fn admit_turn_start(&self) -> Option<Box<dyn Send>> {
        self.try_admit()
            .map(|permit| Box::new(permit) as Box<dyn Send>)
    }
}

impl CoreWorkAdmission for TurnAdmission {
    fn acquire_work_start(
        &self,
        _kind: CoreWorkKind,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Box<dyn Send>>> + Send + '_>>
    {
        Box::pin(async move {
            self.try_admit()
                .map(|permit| Box::new(permit) as Box<dyn Send>)
        })
    }
}

impl Drop for TurnPermit {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active -= 1;
        self.0.active_tx.send_replace(state.active);
    }
}

#[cfg(test)]
#[path = "turn_admission_tests.rs"]
mod tests;
