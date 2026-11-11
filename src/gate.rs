//! The shared gate vocabulary for actions that check before they run.
//!
//! A gate is one observed check. `Pass` means the check succeeded, `Warn` means
//! it did not block but its input was incomplete, and `Fail` means it blocked.
//! A gate after a failure is absent: it did not run.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub name: &'static str,
    pub state: GateState,
    pub detail: String,
}

impl Gate {
    pub fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            state: GateState::Pass,
            detail: detail.into(),
        }
    }

    pub fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            state: GateState::Warn,
            detail: detail.into(),
        }
    }

    pub fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            state: GateState::Fail,
            detail: detail.into(),
        }
    }
}
