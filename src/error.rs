use std::fmt;

/// Why a command stopped.
///
/// `Blocked` is a gate: the run was refused on purpose and the user can act on
/// it. `Failed` is an execution error the tool did not control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Blocked(String),
    Failed(String),
}

impl Error {
    pub fn blocked(message: impl Into<String>) -> Self {
        Self::Blocked(message.into())
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    /// Process exit code, per NEXT.md D6.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Blocked(_) => 1,
            Self::Failed(_) => 2,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocked(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn blocked_exits_1() {
        assert_eq!(Error::blocked("pacman is running").exit_code(), 1);
    }

    #[test]
    fn failed_exits_2() {
        assert_eq!(Error::failed("pacman exited 1").exit_code(), 2);
    }

    #[test]
    fn display_prints_the_message_without_a_prefix() {
        assert_eq!(Error::blocked("disk low").to_string(), "disk low");
    }
}
