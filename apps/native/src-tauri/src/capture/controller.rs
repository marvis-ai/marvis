//! `controller` — the pure capture-lifecycle decision contract.
//!
//! `CaptureLifecycle` records whether a capture is running and answers
//! the two idempotency questions the shared start/stop boundary asks:
//! should `start` create a capture, and should `stop` tear one down.
//! The contract is platform-pure so the decision table is unit-tested
//! without a live macOS capture session.

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct CaptureLifecycle {
    running: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StartDecision {
    Create,
    Noop,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StopDecision {
    Stop,
    Noop,
}

impl CaptureLifecycle {
    pub(crate) fn start_decision(&self) -> StartDecision {
        if self.running {
            StartDecision::Noop
        } else {
            StartDecision::Create
        }
    }

    pub(crate) fn stop_decision(&self) -> StopDecision {
        if self.running {
            StopDecision::Stop
        } else {
            StopDecision::Noop
        }
    }

    pub(crate) fn mark_running(&mut self) {
        self.running = true;
    }
    pub(crate) fn mark_stopped(&mut self) {
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_is_idempotent_when_capture_is_running() {
        let mut state = CaptureLifecycle::default();
        assert_eq!(state.start_decision(), StartDecision::Create);
        state.mark_running();
        assert_eq!(state.start_decision(), StartDecision::Noop);
    }

    #[test]
    fn stop_is_idempotent_when_capture_is_stopped() {
        let mut state = CaptureLifecycle::default();
        assert_eq!(state.stop_decision(), StopDecision::Noop);
        state.mark_running();
        assert_eq!(state.stop_decision(), StopDecision::Stop);
    }
}
