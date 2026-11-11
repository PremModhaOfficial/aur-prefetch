use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::thread;

use crate::error::Error;

/// Result of one external command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutput {
    /// Exit status. A signal death is reported as -1.
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Which pipe a streamed line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Stdout,
    Stderr,
}

/// One line of child output, delivered while the child still runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamLine {
    pub kind: StreamKind,
    pub line: String,
}

/// Every external command goes through this trait.
///
/// A non-zero exit status is not an error: tools such as `pacman -Qu` use it to
/// mean "nothing to report". Callers inspect `code`.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, Error>;

    /// Run with inherited stdio so an interactive tool keeps the terminal.
    /// Returns the exit code; nothing is captured.
    fn run_interactive(&self, program: &str, args: &[&str]) -> Result<i32, Error>;

    /// Run with captured stdio, delivering each line to `on_line` as it
    /// arrives. The return value still carries the full captured streams and
    /// the exit code, so a caller can fold both the live lines and the result.
    ///
    /// This is the one streaming seam. The default falls back to [`Self::run`]
    /// and replays the captured lines after the child exits; `SystemRunner`
    /// streams a real child. `TUI.md` T5 permits this method only for the
    /// capture-only fetch, never for an attached `yay` or `pacdiff`.
    fn run_streaming(
        &self,
        program: &str,
        args: &[&str],
        on_line: &mut dyn FnMut(StreamLine),
    ) -> Result<CommandOutput, Error> {
        let output = self.run(program, args)?;
        for line in output.stdout.lines() {
            on_line(StreamLine {
                kind: StreamKind::Stdout,
                line: line.to_string(),
            });
        }
        for line in output.stderr.lines() {
            on_line(StreamLine {
                kind: StreamKind::Stderr,
                line: line.to_string(),
            });
        }
        Ok(output)
    }
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

    fn run_interactive(&self, program: &str, args: &[&str]) -> Result<i32, Error> {
        let status = Command::new(program)
            .args(args)
            .status()
            .map_err(|err| Error::failed(format!("failed to run {program}: {err}")))?;
        Ok(status.code().unwrap_or(-1))
    }

    fn run_streaming(
        &self,
        program: &str,
        args: &[&str],
        on_line: &mut dyn FnMut(StreamLine),
    ) -> Result<CommandOutput, Error> {
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| Error::failed(format!("failed to run {program}: {err}")))?;

        let (sender, receiver) = mpsc::channel::<StreamLine>();
        if let Some(stdout) = child.stdout.take() {
            let sender = sender.clone();
            thread::spawn(move || stream_pipe(stdout, StreamKind::Stdout, sender));
        }
        if let Some(stderr) = child.stderr.take() {
            let sender = sender.clone();
            thread::spawn(move || stream_pipe(stderr, StreamKind::Stderr, sender));
        }
        drop(sender);

        let mut stdout = String::new();
        let mut stderr = String::new();
        for line in receiver {
            match line.kind {
                StreamKind::Stdout => {
                    stdout.push_str(&line.line);
                    stdout.push('\n');
                }
                StreamKind::Stderr => {
                    stderr.push_str(&line.line);
                    stderr.push('\n');
                }
            }
            on_line(line);
        }

        let status = child
            .wait()
            .map_err(|err| Error::failed(format!("failed to wait for {program}: {err}")))?;
        Ok(CommandOutput {
            code: status.code().unwrap_or(-1),
            stdout,
            stderr,
        })
    }
}

fn stream_pipe<R: Read>(pipe: R, kind: StreamKind, sender: mpsc::Sender<StreamLine>) {
    for line in BufReader::new(pipe).lines() {
        let Ok(line) = line else {
            break;
        };
        if sender.send(StreamLine { kind, line }).is_err() {
            break;
        }
    }
}

pub type Call = (String, Vec<String>);

/// One queued fake stream: an exit code plus the lines to replay.
#[derive(Debug, Default)]
struct StreamScript {
    code: i32,
    lines: Vec<StreamLine>,
}

/// Records every call and replays queued outputs in call order.
#[derive(Debug, Default)]
pub struct FakeRunner {
    calls: Mutex<Vec<Call>>,
    outputs: Mutex<VecDeque<CommandOutput>>,
    interactive: Mutex<VecDeque<i32>>,
    streams: Mutex<VecDeque<StreamScript>>,
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

    /// Queue one interactive exit code. An empty queue returns 0.
    pub fn push_interactive(&self, code: i32) {
        self.interactive
            .lock()
            .expect("fake runner interactive queue poisoned")
            .push_back(code);
    }

    /// Queue one streamed call. stdout lines are replayed before stderr lines,
    /// and the returned [`CommandOutput`] carries the same text.
    pub fn push_stream(&self, code: i32, stdout: &[&str], stderr: &[&str]) {
        let mut lines = Vec::new();
        lines.extend(stdout.iter().map(|line| StreamLine {
            kind: StreamKind::Stdout,
            line: (*line).to_string(),
        }));
        lines.extend(stderr.iter().map(|line| StreamLine {
            kind: StreamKind::Stderr,
            line: (*line).to_string(),
        }));
        self.streams
            .lock()
            .expect("fake runner stream queue poisoned")
            .push_back(StreamScript { code, lines });
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

    fn run_interactive(&self, program: &str, args: &[&str]) -> Result<i32, Error> {
        self.calls
            .lock()
            .expect("fake runner call log poisoned")
            .push((
                program.to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            ));

        Ok(self
            .interactive
            .lock()
            .expect("fake runner interactive queue poisoned")
            .pop_front()
            .unwrap_or(0))
    }

    fn run_streaming(
        &self,
        program: &str,
        args: &[&str],
        on_line: &mut dyn FnMut(StreamLine),
    ) -> Result<CommandOutput, Error> {
        self.calls
            .lock()
            .expect("fake runner call log poisoned")
            .push((
                program.to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            ));

        let script = self
            .streams
            .lock()
            .expect("fake runner stream queue poisoned")
            .pop_front()
            .unwrap_or_default();
        let mut output = CommandOutput {
            code: script.code,
            ..CommandOutput::default()
        };
        for line in script.lines {
            match line.kind {
                StreamKind::Stdout => {
                    output.stdout.push_str(&line.line);
                    output.stdout.push('\n');
                }
                StreamKind::Stderr => {
                    output.stderr.push_str(&line.line);
                    output.stderr.push('\n');
                }
            }
            on_line(line);
        }
        Ok(output)
    }
}

impl<T: CommandRunner + ?Sized> CommandRunner for std::sync::Arc<T> {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, Error> {
        (**self).run(program, args)
    }

    fn run_interactive(&self, program: &str, args: &[&str]) -> Result<i32, Error> {
        (**self).run_interactive(program, args)
    }

    fn run_streaming(
        &self,
        program: &str,
        args: &[&str],
        on_line: &mut dyn FnMut(StreamLine),
    ) -> Result<CommandOutput, Error> {
        (**self).run_streaming(program, args, on_line)
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandOutput, CommandRunner, FakeRunner, StreamKind, SystemRunner};
    use crate::error::Error;

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
    fn fake_runner_records_interactive_calls_and_replays_codes() {
        let runner = FakeRunner::new();
        runner.push_interactive(3);

        let first = runner.run_interactive("yay", &["-Syuu"]).expect("fake run");
        let second = runner.run_interactive("yay", &["-Syuu"]).expect("fake run");

        assert_eq!(first, 3);
        assert_eq!(second, 0);
        assert_eq!(
            runner.calls(),
            vec![
                ("yay".to_string(), vec!["-Syuu".to_string()]),
                ("yay".to_string(), vec!["-Syuu".to_string()]),
            ]
        );
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

    #[test]
    fn system_runner_streams_lines_and_keeps_the_captured_streams() {
        let mut lines = Vec::new();
        let output = SystemRunner
            .run_streaming(
                "sh",
                &["-c", "printf 'one\\ntwo\\n'; printf 'warn\\n' >&2; exit 7"],
                &mut |line| lines.push(line),
            )
            .expect("stream sh");

        assert_eq!(output.code, 7);
        assert_eq!(output.stdout, "one\ntwo\n");
        assert_eq!(output.stderr, "warn\n");
        assert_eq!(lines.len(), 3);
        let stdout: Vec<&str> = lines
            .iter()
            .filter(|line| line.kind == StreamKind::Stdout)
            .map(|line| line.line.as_str())
            .collect();
        let stderr: Vec<&str> = lines
            .iter()
            .filter(|line| line.kind == StreamKind::Stderr)
            .map(|line| line.line.as_str())
            .collect();
        // The two pipes race by nature; each pipe keeps its own order.
        assert_eq!(stdout, ["one", "two"]);
        assert_eq!(stderr, ["warn"]);
    }

    #[test]
    fn fake_runner_scripts_streamed_lines_and_exit_code() {
        let runner = FakeRunner::new();
        runner.push_stream(3, &["downloading", "done"], &["warning"]);

        let mut lines = Vec::new();
        let output = runner
            .run_streaming("sudo", &["-n", "pacman", "-Syuw"], &mut |line| {
                lines.push(line)
            })
            .expect("stream");

        assert_eq!(output.code, 3);
        assert_eq!(output.stdout, "downloading\ndone\n");
        assert_eq!(output.stderr, "warning\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(
            runner.calls(),
            vec![(
                "sudo".to_string(),
                vec!["-n".to_string(), "pacman".to_string(), "-Syuw".to_string()],
            )]
        );
    }

    struct CaptureOnly;

    impl CommandRunner for CaptureOnly {
        fn run(&self, _program: &str, _args: &[&str]) -> Result<CommandOutput, Error> {
            Ok(CommandOutput {
                code: 0,
                stdout: "a\nb\n".to_string(),
                stderr: "e\n".to_string(),
            })
        }

        fn run_interactive(&self, _program: &str, _args: &[&str]) -> Result<i32, Error> {
            Ok(0)
        }
    }

    #[test]
    fn the_default_stream_method_replays_the_captured_output() {
        let mut lines = Vec::new();
        let output = CaptureOnly
            .run_streaming("tool", &[], &mut |line| lines.push(line))
            .expect("default stream");

        assert_eq!(output.stdout, "a\nb\n");
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.kind, line.line.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (StreamKind::Stdout, "a"),
                (StreamKind::Stdout, "b"),
                (StreamKind::Stderr, "e"),
            ]
        );
    }
}
