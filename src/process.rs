use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// OS-native command input. It is not Debug to avoid exposing secret arguments.
pub struct CommandRequest {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub enum IoMode {
    Capture,
    Inherit,
}

/// A launch error is distinct from an unsuccessful exit in the returned Output.
pub trait ProcessRunner {
    fn run(&self, request: &CommandRequest, mode: IoMode) -> io::Result<Output>;
}

/// Native execution with an explicit environment, including an explicit PATH.
pub struct SystemRunner {
    environment: BTreeMap<OsString, OsString>,
}

impl SystemRunner {
    pub fn new(environment: BTreeMap<OsString, OsString>) -> Self {
        Self { environment }
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
            .envs(&self.environment);
        match mode {
            IoMode::Capture => {
                command.stdin(Stdio::null());
            }
            IoMode::Inherit => {
                command
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
            }
        }
        command.output()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

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
