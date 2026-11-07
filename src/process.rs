use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;

/// OS-native command input. It is not Debug to avoid exposing secret arguments.
pub struct CommandRequest {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum IoMode {
    Capture,
    /// Capture output while retaining the foreground process group. This is
    /// intended for elevation adapters whose credentials may change; only the
    /// direct child is supervised and OS permissions can prevent termination.
    ForegroundCapture,
    Inherit,
}

/// Unsuccessful exits are returned in Output. IO errors may also occur after
/// launch (timeout, output limits, cancellation or cleanup), and never prove
/// that no effects occurred.
pub trait ProcessRunner {
    fn run(&self, request: &CommandRequest, mode: IoMode) -> io::Result<Output>;
}

/// Explicit command budgets: by default 30 minutes and 8 MiB of captured bytes.
///
/// The output budget combines stdout/stderr. Zero bytes is valid; zero time is
/// rejected before launch. Kernel spawn/reap can delay supervisor cleanup.
#[derive(Debug, Clone, Copy)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30 * 60),
            max_output_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Native execution with a captured environment and bounded Linux/macOS supervision.
///
/// Captured jobs have an owned process group; timeout/cancellation signals that
/// group and reaps the direct child. Inherit mode preserves terminal job control
/// and can terminate only its direct child. This is not a sandbox or crash
/// recovery; escaped sessions, privilege changes and external child reapers are
/// outside this ownership contract. Other native platforms return an unsupported result.
#[derive(Clone)]
pub struct SystemRunner {
    environment: Arc<BTreeMap<OsString, OsString>>,
    limits: ProcessLimits,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    registry: Arc<unix::Registry>,
}

impl SystemRunner {
    pub fn new(environment: BTreeMap<OsString, OsString>) -> Self {
        Self {
            environment: Arc::new(environment),
            limits: ProcessLimits::default(),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            registry: Arc::default(),
        }
    }

    /// Change this clone's budgets while retaining its shared cancellation domain.
    pub fn with_limits(mut self, limits: ProcessLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Override only this clone's child environment; never mutate process globals.
    pub fn with_environment_overrides(
        mut self,
        variables: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        Arc::make_mut(&mut self.environment).extend(variables);
        self
    }

    /// Remove inherited child-only variables without changing the cancellation domain.
    pub fn without_environment_variables(
        mut self,
        names: impl IntoIterator<Item = OsString>,
    ) -> Self {
        let environment = Arc::make_mut(&mut self.environment);
        for name in names {
            environment.remove(&name);
        }
        self
    }

    /// Cancel all current jobs shared by clones, and reject subsequent jobs.
    pub fn cancel(&self) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            self.registry.cancel()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Native cancellation is not implemented on this platform",
            ))
        }
    }

    /// Wait for registered direct children to be retired and reaped.
    /// This is not evidence about escaped sessions or a persisted finish receipt.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            self.registry.wait_idle(timeout)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = timeout;
            true
        }
    }
}

impl ProcessRunner for SystemRunner {
    fn run(&self, request: &CommandRequest, mode: IoMode) -> io::Result<Output> {
        if std::path::Path::new(&request.program).components().count() == 1
            && crate::app::lookup_variable(&self.environment, "PATH").is_none()
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Executable search requires an explicit PATH",
            ));
        }
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(&request.working_dir)
            .env_clear()
            .envs(self.environment.as_ref());
        match mode {
            IoMode::Capture | IoMode::ForegroundCapture => {
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
            }
            IoMode::Inherit => {
                command
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
            }
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            unix::run(
                &mut command,
                matches!(mode, IoMode::Capture),
                self.limits,
                &self.registry,
            )
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Bounded native execution is not implemented on this platform",
            ))
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn child_overrides_are_local_and_foreground_capture_preserves_group() {
        let temp = tempfile::TempDir::new().unwrap();
        let runner = SystemRunner::new(BTreeMap::from([("FIXTURE".into(), "original".into())]));
        let overridden = runner
            .clone()
            .with_environment_overrides([("FIXTURE".into(), "override".into())]);
        let request = CommandRequest {
            program: "/usr/bin/env".into(),
            args: vec![],
            working_dir: temp.path().into(),
        };
        assert_eq!(
            overridden
                .run(&request, IoMode::ForegroundCapture)
                .unwrap()
                .stdout,
            b"FIXTURE=override\n"
        );
        assert_eq!(
            runner.run(&request, IoMode::Capture).unwrap().stdout,
            b"FIXTURE=original\n"
        );
        let request = CommandRequest {
            program: "/bin/ps".into(),
            args: vec![
                "-o".into(),
                "pgid=".into(),
                "-p".into(),
                std::process::id().to_string().into(),
            ],
            working_dir: temp.path().into(),
        };
        let group = runner.run(&request, IoMode::Capture).unwrap().stdout;
        let request = CommandRequest {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec /bin/ps -o pgid= -p $$".into()],
            working_dir: temp.path().into(),
        };
        assert_eq!(
            runner
                .run(&request, IoMode::ForegroundCapture)
                .unwrap()
                .stdout,
            group
        );
        assert_ne!(runner.run(&request, IoMode::Capture).unwrap().stdout, group);
        overridden.cancel().unwrap();
        assert_eq!(
            runner.run(&request, IoMode::Capture).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn native_process_gets_only_explicit_environment_and_preserves_failure_status() {
        let temp = tempfile::TempDir::new().unwrap();
        let runner = SystemRunner::new(BTreeMap::from([("FIXTURE".into(), "value".into())]));
        let output = runner
            .run(
                &CommandRequest {
                    program: "/usr/bin/env".into(),
                    args: vec![],
                    working_dir: temp.path().to_path_buf(),
                },
                IoMode::Capture,
            )
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"FIXTURE=value\n");
        let output = runner
            .run(
                &CommandRequest {
                    program: "/usr/bin/false".into(),
                    args: vec![],
                    working_dir: temp.path().to_path_buf(),
                },
                IoMode::Capture,
            )
            .unwrap();
        assert!(!output.status.success());
        assert!(
            runner
                .run(
                    &CommandRequest {
                        program: "env".into(),
                        args: vec![],
                        working_dir: temp.path().to_path_buf(),
                    },
                    IoMode::Capture,
                )
                .is_err()
        );
        assert!(
            runner
                .run(
                    &CommandRequest {
                        program: temp.path().join("missing").into_os_string(),
                        args: vec![],
                        working_dir: temp.path().to_path_buf(),
                    },
                    IoMode::Capture,
                )
                .is_err()
        );
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod supervision_tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    const TREE: &str = include_str!("../tests/fixtures/process-tree.sh");
    const OUTPUT: &str = include_str!("../tests/fixtures/process-output.sh");

    fn shell(root: &std::path::Path, script: &str, args: &[OsString]) -> CommandRequest {
        let mut all = vec!["-c".into(), script.into(), "fixture".into()];
        all.extend_from_slice(args);
        CommandRequest {
            program: "/bin/sh".into(),
            args: all,
            working_dir: root.to_path_buf(),
        }
    }
    fn runner(timeout: Duration, max_output_bytes: usize) -> SystemRunner {
        SystemRunner::new(BTreeMap::new()).with_limits(ProcessLimits {
            timeout,
            max_output_bytes,
        })
    }
    fn assert_no_live_process(pid: &str) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let output = Command::new("/bin/ps")
                .env_clear()
                .args(["-o", "stat=", "-p", pid])
                .output()
                .unwrap();
            let state = String::from_utf8(output.stdout).unwrap();
            if (output.status.code() == Some(1) && state.is_empty() && output.stderr.is_empty())
                || (output.status.success() && state.trim_start().starts_with('Z'))
            {
                return;
            }
            assert!(output.status.success(), "process observation failed");
            assert!(
                Instant::now() < deadline,
                "fixture process remained live: {pid}: {state}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn assert_tree_stopped(path: &std::path::Path) {
        let pids = fs::read_to_string(path).unwrap();
        assert_eq!(pids.lines().count(), 2);
        for pid in pids.lines() {
            assert_no_live_process(pid);
        }
    }

    #[test]
    fn captured_output_has_one_combined_limit_without_successful_truncation() {
        let root = tempfile::TempDir::new().unwrap();
        let request = shell(root.path(), OUTPUT, &["4".into(), "4".into()]);
        let result = runner(Duration::from_secs(2), 8)
            .run(&request, IoMode::Capture)
            .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, [0; 4]);
        assert_eq!(result.stderr, [0; 4]);
        let error = runner(Duration::from_secs(2), 7)
            .run(&request, IoMode::Capture)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    }

    #[test]
    fn default_capture_rejects_more_than_eight_mib() {
        let root = tempfile::TempDir::new().unwrap();
        let result = SystemRunner::new(BTreeMap::new()).run(
            &CommandRequest {
                program: "/usr/bin/head".into(),
                args: ["-c", "8388609", "/dev/zero"]
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                working_dir: root.path().to_path_buf(),
            },
            IoMode::Capture,
        );
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    }

    #[test]
    fn large_legitimate_output_on_both_pipes_does_not_deadlock() {
        let root = tempfile::TempDir::new().unwrap();
        let request = shell(root.path(), OUTPUT, &["1048576".into(), "1048576".into()]);
        let output = runner(Duration::from_secs(5), 2 * 1024 * 1024)
            .run(&request, IoMode::Capture)
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 1024 * 1024);
        assert_eq!(output.stderr.len(), 1024 * 1024);
    }

    #[test]
    fn timeout_stops_captured_children_and_reaps_the_direct_child() {
        let root = tempfile::TempDir::new().unwrap();
        let pids = root.path().join("pids");
        let runner = runner(Duration::from_millis(500), 1024);
        let result = runner.run(
            &shell(root.path(), TREE, &[pids.clone().into_os_string()]),
            IoMode::Capture,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(runner.wait_idle(Duration::ZERO));
        assert_tree_stopped(&pids);
    }

    #[test]
    fn successful_exit_does_not_leave_a_captured_background_child_or_open_pipe() {
        let root = tempfile::TempDir::new().unwrap();
        let pids = root.path().join("pids");
        let runner = runner(Duration::from_secs(2), 1024);
        let result = runner
            .run(
                &shell(
                    root.path(),
                    TREE,
                    &[pids.clone().into_os_string(), "leave".into()],
                ),
                IoMode::Capture,
            )
            .unwrap();
        assert!(result.status.success());
        assert!(runner.wait_idle(Duration::ZERO));
        assert_tree_stopped(&pids);
    }

    #[test]
    fn cancellation_is_shared_by_clones_and_rejects_future_jobs() {
        let root = tempfile::TempDir::new().unwrap();
        let pids = root.path().join("pids");
        let runner = runner(Duration::from_secs(2), 1024);
        let worker_runner = runner.clone();
        let request = shell(root.path(), TREE, &[pids.clone().into_os_string()]);
        let worker = std::thread::spawn(move || worker_runner.run(&request, IoMode::Capture));
        let deadline = Instant::now() + Duration::from_secs(1);
        while !fs::read_to_string(&pids).is_ok_and(|contents| contents.lines().count() == 2) {
            assert!(Instant::now() < deadline, "fixture failed to start");
            std::thread::sleep(Duration::from_millis(5));
        }
        runner.cancel().unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(runner.wait_idle(Duration::from_secs(1)));
        assert_tree_stopped(&pids);
        assert_eq!(
            runner
                .run(&shell(root.path(), "exit 0", &[]), IoMode::Capture)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn zero_output_is_valid_but_zero_time_fails_before_a_command_starts() {
        let root = tempfile::TempDir::new().unwrap();
        let request = shell(root.path(), "exit 0", &[]);
        assert!(
            runner(Duration::from_secs(1), 0)
                .run(&request, IoMode::Capture)
                .unwrap()
                .status
                .success()
        );
        assert_eq!(
            runner(Duration::ZERO, 1024)
                .run(&request, IoMode::Capture)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
