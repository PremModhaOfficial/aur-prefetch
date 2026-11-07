use std::collections::VecDeque;
use std::process::Command;
use std::sync::Mutex;

use crate::error::Error;

/// Result of one external command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutput {
    /// Exit status. A signal death is reported as -1.
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Every external command goes through this trait.
///
/// A non-zero exit status is not an error: tools such as `pacman -Qu` use it to
/// mean "nothing to report". Callers inspect `code`.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, Error>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, Error> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|err| Error::failed(format!("failed to run {program}: {err}")))?;

        Ok(CommandOutput {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

pub type Call = (String, Vec<String>);

/// Records every call and replays queued outputs in call order.
#[derive(Debug, Default)]
pub struct FakeRunner {
    calls: Mutex<Vec<Call>>,
    outputs: Mutex<VecDeque<CommandOutput>>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue one output. A call with an empty queue returns a silent success.
    pub fn push_output(&self, output: CommandOutput) {
        self.outputs
            .lock()
            .expect("fake runner output queue poisoned")
            .push_back(output);
    }

    /// Every call in order, as `(program, args)`.
    pub fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .expect("fake runner call log poisoned")
            .clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, Error> {
        self.calls
            .lock()
            .expect("fake runner call log poisoned")
            .push((
                program.to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            ));

        Ok(self
            .outputs
            .lock()
            .expect("fake runner output queue poisoned")
            .pop_front()
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandOutput, CommandRunner, FakeRunner, SystemRunner};

    #[test]
    fn fake_runner_records_the_exact_argv() {
        let runner = FakeRunner::new();
        runner.run("/usr/bin/pacman", &["-Qui"]).expect("fake run");
        runner.run("pacman-conf", &["CacheDir"]).expect("fake run");

        assert_eq!(
            runner.calls(),
            vec![
                ("/usr/bin/pacman".to_string(), vec!["-Qui".to_string()],),
                ("pacman-conf".to_string(), vec!["CacheDir".to_string()],),
            ]
        );
    }

    #[test]
    fn fake_runner_replays_queued_outputs_in_order() {
        let runner = FakeRunner::new();
        runner.push_output(CommandOutput {
            code: 1,
            stdout: "first".to_string(),
            stderr: String::new(),
        });

        let first = runner.run("pacman", &["-Qu"]).expect("fake run");
        let second = runner.run("pacman", &["-Qu"]).expect("fake run");

        assert_eq!(first.code, 1);
        assert_eq!(first.stdout, "first");
        assert_eq!(second, CommandOutput::default());
    }

    #[test]
    fn system_runner_captures_stdout_code_and_stderr() {
        let output = SystemRunner
            .run("sh", &["-c", "printf out; printf err >&2; exit 7"])
            .expect("run sh");

        assert_eq!(output.code, 7);
        assert_eq!(output.stdout, "out");
        assert_eq!(output.stderr, "err");
    }
}
