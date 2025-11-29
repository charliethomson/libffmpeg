use std::{
    ops::ControlFlow,
    process::{ExitStatus, Stdio},
    sync::Arc,
};

use liberror::AnyError;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, BufReader, Lines},
    process::Command,
};
use tokio_util::sync::CancellationToken;
use tracing::Level;
use valuable::Valuable;

use crate::util::{exit::CommandExitCode, read::reader_or_never};

#[derive(Debug, Clone, Serialize, Deserialize, Valuable, Error)]
pub enum CommandError {
    #[error("cancellation requested")]
    Cancelled,

    #[error("failed to spawn: {inner_error}")]
    BadSpawn { inner_error: AnyError },

    #[error("exited unsuccessfully: {inner_error}")]
    BadExit { inner_error: AnyError },

    #[error("failed to acquire permit: {inner_error}")]
    Acquire { inner_error: AnyError },
}

#[derive(Debug, Clone, Serialize, Deserialize, Valuable)]
pub struct CommandExit {
    pub stdout_lines: Vec<String>,
    pub stderr_lines: Vec<String>,
    pub exit_code: Option<CommandExitCode>,
}

#[derive(Clone, Debug, Valuable)]
pub struct CommandMonitorSender {
    #[valuable(skip)]
    stdout_tx: Arc<tokio::sync::mpsc::Sender<String>>,
    #[valuable(skip)]
    stderr_tx: Arc<tokio::sync::mpsc::Sender<String>>,
}
impl CommandMonitorSender {
    fn new(
        stdout_tx: tokio::sync::mpsc::Sender<String>,
        stderr_tx: tokio::sync::mpsc::Sender<String>,
    ) -> Self {
        Self {
            stdout_tx: Arc::new(stdout_tx),
            stderr_tx: Arc::new(stderr_tx),
        }
    }
}

#[derive(Debug)]
pub enum CommandMonitorMessage {
    Stdout { line: String },
    Stderr { line: String },
}

pub struct CommandMonitorReceiver {
    stdout_rx: tokio::sync::mpsc::Receiver<String>,
    stderr_rx: tokio::sync::mpsc::Receiver<String>,
}
impl CommandMonitorReceiver {
    pub async fn recv(&mut self) -> Option<CommandMonitorMessage> {
        tokio::select! {
            delivery = self.stdout_rx.recv() => {
                delivery.map(|line| CommandMonitorMessage::Stdout { line })
            }
            delivery = self.stderr_rx.recv() => {
                delivery.map(|line| CommandMonitorMessage::Stderr { line })
            }
        }
    }

    fn new(
        stdout_rx: tokio::sync::mpsc::Receiver<String>,
        stderr_rx: tokio::sync::mpsc::Receiver<String>,
    ) -> Self {
        Self {
            stdout_rx,
            stderr_rx,
        }
    }
}

pub struct CommandMonitor {
    pub sender: CommandMonitorSender,
    pub receiver: CommandMonitorReceiver,
}
impl Default for CommandMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandMonitor {
    #[must_use] pub fn with_capacity(capacity: usize) -> Self {
        let (stdout_tx, stdout_rx) = tokio::sync::mpsc::channel(capacity);
        let (stderr_tx, stderr_rx) = tokio::sync::mpsc::channel(capacity);

        let sender = CommandMonitorSender::new(stdout_tx, stderr_tx);
        let receiver = CommandMonitorReceiver::new(stdout_rx, stderr_rx);

        Self { sender, receiver }
    }
    #[must_use] pub fn new() -> Self {
        Self::with_capacity(100)
    }
}

#[derive(Valuable)]
struct CommandContext<
    StdoutReader: AsyncBufRead + Unpin + Send,
    StderrReader: AsyncBufRead + Unpin + Send,
> {
    #[valuable(skip)]
    child: tokio::process::Child,
    #[valuable(skip)]
    stdout: Lines<StdoutReader>,
    #[valuable(skip)]
    stderr: Lines<StderrReader>,
    #[valuable(skip)]
    cancellation_token: CancellationToken,

    sender: Option<CommandMonitorSender>,
    result: CommandExit,
}
impl<StdoutReader: AsyncBufRead + Unpin + Send, StderrReader: AsyncBufRead + Unpin + Send>
    CommandContext<StdoutReader, StderrReader>
{
    fn new(
        child: tokio::process::Child,
        sender: Option<CommandMonitorSender>,
        stdout: Lines<StdoutReader>,
        stderr: Lines<StderrReader>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            child,
            stdout,
            stderr,
            sender,
            cancellation_token,

            result: CommandExit {
                stdout_lines: Vec::new(),
                stderr_lines: Vec::new(),
                exit_code: None,
            },
        }
    }

    #[tracing::instrument(level=Level::DEBUG, "command_context::on_exited", skip(self))]
    fn on_exited(
        &mut self,
        exit_result: Result<ExitStatus, std::io::Error>,
    ) -> ControlFlow<Result<CommandExit, CommandError>> {
        match exit_result {
            Ok(status) => {
                self.result.exit_code = Some(status.into());
                if status.success() {
                    tracing::trace!("command process completed successfully");
                } else {
                    tracing::error!(
                        exit_code = ?status.code(),
                        stderr_lines = ?self.result.stderr_lines,
                        "command process completed with non-zero exit code"
                    );
                }
                ControlFlow::Break(Ok(self.result.clone()))
            }
            Err(e) => {
                tracing::error!(error = %e, "command process wait failed");
                ControlFlow::Break(Err(CommandError::BadExit {
                    inner_error: e.into(),
                }))
            }
        }
    }

    #[tracing::instrument(level=Level::DEBUG, "command_context::on_cancelled", skip(self))]
    async fn on_cancelled(&mut self) -> ControlFlow<Result<CommandExit, CommandError>> {
        tracing::warn!("Cancellation requested, terminating command process");
        self.child.kill().await.expect("Failed to kill ffmpeg");
        return ControlFlow::Break(Err(CommandError::Cancelled));
    }

    #[tracing::instrument(level=Level::DEBUG, "command_context::on_stdout_line", skip(self))]
    async fn on_stdout_line(
        &mut self,
        line: String,
    ) -> ControlFlow<Result<CommandExit, CommandError>> {
        self.result.stdout_lines.push(line.clone());
        tracing::debug!(line = line, "command wrote to stdout");
        let Some(sender) = self.sender.clone() else {
            return ControlFlow::Continue(());
        };

        if let Err(e) = sender.stdout_tx.send(line).await {
            tracing::error!(error =% e, error_context =? e, "Failed to write stdout line to channel");
        }

        ControlFlow::Continue(())
    }

    #[tracing::instrument(level=Level::DEBUG, "command_context::on_stderr_line", skip(self))]
    async fn on_stderr_line(
        &mut self,
        line: String,
    ) -> ControlFlow<Result<CommandExit, CommandError>> {
        self.result.stderr_lines.push(line.clone());
        tracing::debug!(line = line, "command wrote to stderr");
        let Some(sender) = self.sender.clone() else {
            return ControlFlow::Continue(());
        };

        if let Err(e) = sender.stderr_tx.send(line).await {
            tracing::error!(error =% e, error_context =? e, "Failed to write stdout line to channel");
        }

        ControlFlow::Continue(())
    }

    #[tracing::instrument(level=Level::DEBUG, "command_context::tick", skip(self))]
    async fn tick(&mut self) -> ControlFlow<Result<CommandExit, CommandError>> {
        tokio::select! {
            exit_result = self.child.wait() => return self.on_exited(exit_result),
            () = self.cancellation_token.cancelled() => return self.on_cancelled().await,
            Ok(Some(line)) = self.stdout.next_line() => return self.on_stdout_line(line).await,
            Ok(Some(line)) = self.stderr.next_line() => return self.on_stderr_line(line).await,
        }
    }
}

#[tracing::instrument("libffmpeg::cmd::run", skip(prepare))]
pub async fn run<Prepare>(
    command: &str,
    sender: Option<CommandMonitorSender>,
    cancellation_token: CancellationToken,
    prepare: Prepare,
) -> Result<CommandExit, CommandError>
where
    Prepare: FnOnce(&mut Command),
{
    let mut cmd = Command::new(command);

    prepare(&mut cmd);

    tracing::info!(args = ?cmd.as_std().get_args().collect::<Vec<_>>(), "Executing command");

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| CommandError::BadSpawn {
        inner_error: e.into(),
    })?;

    let stdout = reader_or_never(child.stdout.take());
    let stdout = BufReader::new(stdout).lines();

    let stderr = reader_or_never(child.stderr.take());
    let stderr = BufReader::new(stderr).lines();

    let mut context = CommandContext::new(child, sender, stdout, stderr, cancellation_token);

    loop {
        if let ControlFlow::Break(result) = context.tick().await {
            return result;
        }
    }
}
