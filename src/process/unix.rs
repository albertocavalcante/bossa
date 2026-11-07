//! Supervision for Unix children. Only capture mode creates an owned process
//! group; inherited terminal IO must retain foreground job-control behavior.

use super::ProcessLimits;
use rustix::process::{self, Pid, Signal, WaitId, WaitIdOptions};
use std::collections::HashSet;
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Output};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Job {
    pid: Pid,
    group: bool,
}

impl Job {
    fn kill(self) -> io::Result<()> {
        // The guard retains the child unreaped until group cleanup finishes.
        let result = if self.group {
            process::kill_process_group(self.pid, Signal::KILL)
        } else {
            process::kill_process(self.pid, Signal::KILL)
        };
        match result {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            // macOS reports EPERM when this owned group has only exited,
            // unsignalable members. Confirm the unreaped leader exited; never
            // apply this exception to a running or externally reaped child.
            #[cfg(target_os = "macos")]
            Err(rustix::io::Errno::PERM) if owned_exit_visible(self.pid) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(target_os = "macos")]
fn owned_exit_visible(pid: Pid) -> bool {
    // killpg can observe the last member exiting slightly before waitid exposes
    // its status. Keep the child unreaped and briefly retry only that proof.
    // A live child, invalid identity, or genuine permission error is never
    // accepted as clean shutdown after the finite observation window.
    for attempt in 0..=20 {
        match process::waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(Some(_)) => return true,
            Ok(None) | Err(rustix::io::Errno::INTR) if attempt < 20 => {
                std::thread::sleep(Duration::from_millis(1));
            }
            _ => return false,
        }
    }
    false
}

#[derive(Default)]
struct State {
    cancelled: bool,
    jobs: HashSet<Job>,
}

#[derive(Default)]
pub(super) struct Registry {
    state: Mutex<State>,
    idle: Condvar,
}

impl Registry {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn cancel(&self) -> io::Result<()> {
        let mut state = self.lock();
        state.cancelled = true;
        let mut error = None;
        for job in &state.jobs {
            if let Err(failure) = job.kill() {
                error = Some(failure);
            }
        }
        error.map_or(Ok(()), Err)
    }

    pub(super) fn wait_idle(&self, timeout: Duration) -> bool {
        let state = self.lock();
        let (state, _) = self
            .idle
            .wait_timeout_while(state, timeout, |state| !state.jobs.is_empty())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.jobs.is_empty()
    }

    fn spawn(
        self: &Arc<Self>,
        command: &mut Command,
        group: bool,
        deadline: Instant,
    ) -> io::Result<OwnedChild> {
        // Keep spawn and registration indivisible with respect to cancellation.
        let mut state = self.lock();
        if state.cancelled {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Native runner has been cancelled",
            ));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Process launch budget expired before spawning",
            ));
        }
        if group {
            command.process_group(0);
        }
        let child = command.spawn()?;
        let pid = Pid::from_child(&child);
        let job = Job { pid, group };
        state.jobs.insert(job);
        Ok(OwnedChild {
            child,
            job,
            registry: Arc::clone(self),
            reaped: false,
        })
    }

    fn cancelled(&self) -> bool {
        self.lock().cancelled
    }
}

struct OwnedChild {
    child: Child,
    job: Job,
    registry: Arc<Registry>,
    reaped: bool,
}

impl OwnedChild {
    fn exited(&mut self) -> io::Result<bool> {
        // NOWAIT retains ownership until cleanup/reap. A recycled PID can
        // never be signalled by a stale registry entry under this contract.
        match process::waitid(
            WaitId::Pid(self.job.pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(status) => Ok(status.is_some()),
            Err(rustix::io::Errno::INTR) => Ok(false),
            Err(error) => {
                if error == rustix::io::Errno::CHILD {
                    // An external reaper broke exclusive ownership. Retire the
                    // identity rather than signal a potentially recycled PID.
                    self.retire();
                }
                Err(error.into())
            }
        }
    }

    fn retire(&mut self) {
        self.registry.lock().jobs.remove(&self.job);
        self.reaped = true;
        self.registry.idle.notify_all();
    }

    fn finish(&mut self) -> io::Result<ExitStatus> {
        // Signal before reaping, while the PID/group identity remains reserved.
        self.job.kill()?;
        let mut state = self.registry.lock();
        state.jobs.remove(&self.job);
        // Removal happens before wait frees the PID; a concurrent canceller
        // cannot retain a stale identity across that point.
        let result = self.child.wait();
        self.reaped = true;
        self.registry.idle.notify_all();
        result
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.finish();
        }
    }
}

fn nonblocking(pipe: &impl AsFd) -> io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(pipe)?;
    rustix::fs::fcntl_setfl(pipe, flags | rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}

fn drain<R: Read>(
    pipe: &mut Option<R>,
    contents: &mut Vec<u8>,
    remaining: &mut usize,
    max_bytes: usize,
) -> io::Result<()> {
    let Some(reader) = pipe else {
        return Ok(());
    };
    let mut buffer = [0_u8; 8192];
    // Yield regularly so continuous stdout cannot starve deadlines or stderr.
    for _ in 0..64 {
        let size = buffer.len().min(remaining.saturating_add(1));
        match reader.read(&mut buffer[..size]) {
            Ok(0) => {
                *pipe = None;
                return Ok(());
            }
            Ok(count) => {
                if count > *remaining {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Captured process output exceeded its {max_bytes} byte limit; output was not accepted as complete"
                        ),
                    ));
                }
                contents.extend_from_slice(&buffer[..count]);
                *remaining -= count;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) fn run(
    command: &mut Command,
    capture: bool,
    limits: ProcessLimits,
    registry: &Arc<Registry>,
) -> io::Result<Output> {
    let deadline = Instant::now()
        .checked_add(limits.timeout)
        .filter(|_| !limits.timeout.is_zero())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "A finite positive process timeout is required",
            )
        })?;
    let mut owned = registry.spawn(command, capture, deadline)?;
    let result = collect_output(&mut owned, deadline, limits, registry);
    match result {
        Err(error) if !owned.reaped => {
            if let Err(cleanup) = owned.finish() {
                return Err(io::Error::other(format!(
                    "{error}; native job cleanup incomplete: {cleanup}"
                )));
            }
            Err(error)
        }
        result => result,
    }
}

fn collect_output(
    owned: &mut OwnedChild,
    deadline: Instant,
    limits: ProcessLimits,
    registry: &Registry,
) -> io::Result<Output> {
    let mut stdout = owned.child.stdout.take();
    let mut stderr = owned.child.stderr.take();
    if let Some(pipe) = &stdout {
        nonblocking(pipe)?;
    }
    if let Some(pipe) = &stderr {
        nonblocking(pipe)?;
    }
    let mut output = Output {
        status: ExitStatus::default(),
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let mut remaining = limits.max_output_bytes;
    let mut completed = false;
    loop {
        if registry.cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Native process execution was cancelled; effects may be partial",
            ));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Native process exceeded its timeout; effects may be partial",
            ));
        }
        drain(
            &mut stdout,
            &mut output.stdout,
            &mut remaining,
            limits.max_output_bytes,
        )?;
        drain(
            &mut stderr,
            &mut output.stderr,
            &mut remaining,
            limits.max_output_bytes,
        )?;
        if !completed && owned.exited()? {
            output.status = owned.finish()?;
            completed = true;
        }
        if completed && stdout.is_none() && stderr.is_none() {
            if registry.cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Native process execution was cancelled; effects may be partial",
                ));
            }
            return Ok(output);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
