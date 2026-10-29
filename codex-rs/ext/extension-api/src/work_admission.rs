//! Lets a host stop new model requests and command launches during shutdown drain.

use std::future::Future;
use std::pin::Pin;

/// New work that a host may gate independently from turn-start admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreWorkKind {
    /// Opening a model response stream.
    ModelRequest,
    /// Starting a new command process.
    CommandProcessStart,
}

/// Host-provided admission for individual Core work starts.
pub trait CoreWorkAdmission: std::fmt::Debug + Send + Sync {
    /// Returns a permit held through completion, or `None` when the host is draining.
    fn acquire_work_start(
        &self,
        kind: CoreWorkKind,
    ) -> Pin<Box<dyn Future<Output = Option<Box<dyn Send>>> + Send + '_>>;
}
