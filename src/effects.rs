use crate::error::{error, Context, Error};
use crate::manifest::{Effect, Selection};
use nix::errno::Errno;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub fn run<'a>(
    effects: impl IntoIterator<Item = &'a Effect>,
    selection: &Selection,
) -> crate::Result<()> {
    let mut results = Vec::new();
    let mut running = Vec::new();
    for effect in effects {
        let result = results.len();
        results.push(None);
        let argv = effect.exec.resolve(selection);
        let (program, arguments) = argv.split_first().unwrap();
        let mut command = Command::new(&**program);
        for argument in arguments {
            command.arg(&**argument);
        }
        command.process_group(0);
        let started = Instant::now();
        let mut child = match command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context(format_args!("effect '{}'", effect.name))
        {
            Ok(child) => child,
            Err(error) => {
                results[result] = Some(Err(error));
                continue;
            }
        };
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        if let Err(error) = nonblocking(&stdout)
            .and_then(|()| nonblocking(&stderr))
            .context(format_args!("effect '{}' output", effect.name))
        {
            let _ = terminate(&mut child);
            results[result] = Some(Err(error));
            continue;
        }
        running.push(RunningEffect {
            result,
            name: &effect.name,
            ignore_failure: effect.ignore_failure,
            process: Some(Process {
                child,
                stdout: Capture::new(stdout),
                stderr: Capture::new(stderr),
                started,
            }),
        });
    }

    let events = PollFlags::POLLIN | PollFlags::POLLHUP | PollFlags::POLLERR;
    let mut ready = vec![(false, false); running.len()];
    while running.iter().any(|effect| effect.process.is_some()) {
        ready.fill((false, false));
        let mut remaining = TIMEOUT;
        let polled = {
            let mut polls = Vec::with_capacity(running.len() * 2);
            for effect in &running {
                let Some(process) = &effect.process else {
                    continue;
                };
                if let Some(stdout) = &process.stdout.stream {
                    polls.push(PollFd::new(stdout.as_fd(), events));
                }
                if let Some(stderr) = &process.stderr.stream {
                    polls.push(PollFd::new(stderr.as_fd(), events));
                }
                remaining = remaining.min(TIMEOUT.saturating_sub(process.started.elapsed()));
            }
            let timeout = PollTimeout::try_from(remaining.min(POLL_INTERVAL)).unwrap();
            let result = poll(&mut polls, timeout);
            if result.is_ok() {
                let mut polls = polls.iter();
                for (index, effect) in running.iter().enumerate() {
                    let Some(process) = &effect.process else {
                        continue;
                    };
                    if process.stdout.stream.is_some() {
                        ready[index].0 = polls
                            .next()
                            .unwrap()
                            .revents()
                            .is_some_and(|events| !events.is_empty());
                    }
                    if process.stderr.stream.is_some() {
                        ready[index].1 = polls
                            .next()
                            .unwrap()
                            .revents()
                            .is_some_and(|events| !events.is_empty());
                    }
                }
            }
            result
        };
        match polled {
            Ok(_) => {}
            Err(Errno::EINTR) => continue,
            result => {
                for effect in &mut running {
                    if let Some(mut process) = effect.process.take() {
                        let _ = terminate(&mut process.child);
                    }
                }
                result.context("effects")?;
            }
        }

        for (index, effect) in running.iter_mut().enumerate() {
            let Some(process) = &mut effect.process else {
                continue;
            };
            let completion = if ready[index].0 {
                process
                    .stdout
                    .drain()
                    .context(format_args!("effect '{}' stdout", effect.name))
                    .err()
                    .map(Completion::Failed)
            } else {
                None
            }
            .or_else(|| {
                if !ready[index].1 {
                    return None;
                }
                process
                    .stderr
                    .drain()
                    .context(format_args!("effect '{}' stderr", effect.name))
                    .err()
                    .map(Completion::Failed)
            });
            let completion = match completion {
                Some(completion) => Some(completion),
                None => match process
                    .child
                    .try_wait()
                    .context(format_args!("effect '{}'", effect.name))
                {
                    Ok(Some(status)) => Some(Completion::Exited(status)),
                    Ok(None) if process.started.elapsed() >= TIMEOUT => Some(Completion::TimedOut),
                    Ok(None) => None,
                    Err(error) => Some(Completion::Failed(error)),
                },
            };
            let Some(completion) = completion else {
                continue;
            };

            let mut process = effect.process.take().unwrap();
            let result = match completion {
                Completion::Exited(status) => {
                    finish(effect.name, process, status, false, effect.ignore_failure)
                }
                Completion::TimedOut => terminate(&mut process.child)
                    .context(format_args!("effect '{}'", effect.name))
                    .and_then(|status| {
                        finish(effect.name, process, status, true, effect.ignore_failure)
                    }),
                Completion::Failed(error) => {
                    let _ = terminate(&mut process.child);
                    Err(error)
                }
            };
            results[effect.result] = Some(result);
        }
    }

    let mut failures = results
        .into_iter()
        .filter_map(|result| result.unwrap().err());
    let Some(first) = failures.next() else {
        return Ok(());
    };
    let Some(second) = failures.next() else {
        return Err(first);
    };
    let mut count = 2;
    let mut details = format!("- {first}\n- {second}");
    for failure in failures {
        count += 1;
        details.push('\n');
        std::fmt::write(&mut details, format_args!("- {failure}")).unwrap();
    }
    Err(error!("{count} effects: {details}"))
}

#[cfg(not(test))]
const TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const TIMEOUT: Duration = Duration::from_millis(500);
const OUTPUT_LIMIT: usize = 16 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct RunningEffect<'a> {
    result: usize,
    name: &'a str,
    ignore_failure: bool,
    process: Option<Process>,
}

struct Process {
    child: Child,
    stdout: Capture<ChildStdout>,
    stderr: Capture<ChildStderr>,
    started: Instant,
}

struct Capture<T> {
    stream: Option<T>,
    output: Vec<u8>,
    truncated: bool,
}

impl<T: AsFd + Read> Capture<T> {
    fn new(stream: T) -> Self {
        Self {
            stream: Some(stream),
            output: Vec::new(),
            truncated: false,
        }
    }

    fn drain(&mut self) -> std::io::Result<()> {
        let mut buffer = [0; 4096];
        let mut remaining = OUTPUT_LIMIT;
        loop {
            let Some(stream) = &mut self.stream else {
                return Ok(());
            };
            let length = buffer.len().min(remaining);
            let read = stream.read(&mut buffer[..length]);
            match read {
                Ok(0) => {
                    self.stream = None;
                    return Ok(());
                }
                Ok(read) => {
                    let retained = read.min(OUTPUT_LIMIT.saturating_sub(self.output.len()));
                    self.output.extend_from_slice(&buffer[..retained]);
                    self.truncated |= retained < read;
                    remaining -= read;
                    if remaining == 0 {
                        return Ok(());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}

enum Completion {
    Exited(ExitStatus),
    TimedOut,
    Failed(Error),
}

fn nonblocking(stream: &impl AsFd) -> nix::Result<()> {
    let flags = OFlag::from_bits_retain(fcntl(stream, FcntlArg::F_GETFL)?);
    fcntl(stream, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))?;
    Ok(())
}

fn terminate(child: &mut Child) -> std::io::Result<ExitStatus> {
    // kill the process group so descendants cannot outlive the timeout
    if let Err(error) = kill(Pid::from_raw(-(child.id() as i32)), Signal::SIGKILL) {
        if error != Errno::ESRCH {
            return Err(error.into());
        }
    }
    child.wait()
}

fn finish(
    name: &str,
    mut process: Process,
    status: ExitStatus,
    timed_out: bool,
    ignore_failure: bool,
) -> crate::Result<()> {
    process
        .stdout
        .drain()
        .context(format_args!("effect '{name}' stdout"))?;
    process
        .stderr
        .drain()
        .context(format_args!("effect '{name}' stderr"))?;
    process.stdout.stream = None;
    process.stderr.stream = None;

    let mut output = String::new();
    append_output(&mut output, "stdout", &process.stdout);
    append_output(&mut output, "stderr", &process.stderr);
    if timed_out {
        if output.is_empty() {
            return Err(error!(
                "effect '{name}' timed out after {} seconds",
                TIMEOUT.as_secs_f32()
            ));
        }
        return Err(error!(
            "effect '{name}' timed out after {} seconds: {output}",
            TIMEOUT.as_secs_f32()
        ));
    }
    if !status.success() && !ignore_failure {
        if output.is_empty() {
            return Err(error!("effect '{name}': {status}"));
        }
        return Err(error!("effect '{name}': {status}: {output}"));
    }
    Ok(())
}

fn append_output<T>(output: &mut String, label: &str, capture: &Capture<T>) {
    let text = String::from_utf8_lossy(&capture.output);
    let text = text.trim();
    if text.is_empty() && !capture.truncated {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(label);
    output.push_str(":\n");
    output.push_str(text);
    if capture.truncated {
        output.push_str("\n[truncated]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    #[test]
    fn bounds_captured_output() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(&vec![b'x'; OUTPUT_LIMIT * 2]).unwrap();
        drop(writer);
        nonblocking(&reader).unwrap();
        let mut capture = Capture::new(reader);

        capture.drain().unwrap();
        capture.drain().unwrap();

        assert_eq!(capture.output.len(), OUTPUT_LIMIT);
        assert!(capture.truncated);
    }
}
