//! Python Inference child process supervisor and IPC communicator.
//! Launches Python with Win32 Job Object limit `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`
//! to guarantee zero zombie processes even upon abrupt termination.

use crate::events::{AppEvent, EventSink};
use nopeek_core::config::{EnvMode, GuiConfig};
use nopeek_core::ipc::{IpcCommand, IpcEvent};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub struct IpcWorker {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    #[cfg(windows)]
    job_handle: Option<HANDLE>,
    child_handle: Arc<Mutex<Option<Child>>>,
}

impl IpcWorker {
    /// Locate the Python interpreter: prefer `.venv/Scripts/python.exe` at the repo root.
    pub fn find_python() -> PathBuf {
        let mut cur = std::env::current_dir().ok();
        while let Some(dir) = cur {
            let venv_py = if cfg!(windows) {
                dir.join(".venv").join("Scripts").join("python.exe")
            } else {
                dir.join(".venv").join("bin").join("python")
            };
            if venv_py.is_file() {
                return venv_py;
            }
            cur = dir.parent().map(Path::to_path_buf);
        }

        if let Ok(venv) = std::env::var("VIRTUAL_ENV") {
            let p = Path::new(&venv)
                .join(if cfg!(windows) { "Scripts" } else { "bin" })
                .join(if cfg!(windows) {
                    "python.exe"
                } else {
                    "python"
                });
            if p.is_file() {
                return p;
            }
        }

        PathBuf::from(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        })
    }

    /// Spawn the Python worker process, bind it to a Win32 Job Object,
    /// and start background reader and monitor threads.
    pub fn spawn(cfg: &GuiConfig, sink: EventSink) -> std::io::Result<Self> {
        let py_bin = Self::find_python();
        let mut cmd = Command::new(&py_bin);
        if let Some(venv_dir) = py_bin.parent().and_then(|p| p.parent()) {
            if let Some(repo_root) = venv_dir.parent() {
                if repo_root.exists() {
                    cmd.current_dir(repo_root);
                }
            }
        }
        cmd.args(["-m", "no_peek.main", "--ipc", "stdio"]);
        if cfg.env_mode == EnvMode::Prod {
            cmd.arg("--headless");
            cmd.stderr(Stdio::null());
        } else {
            cmd.stderr(Stdio::inherit());
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());

        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW = 0x08000000: hide console window in production mode
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            if cfg.env_mode == EnvMode::Prod {
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
        }

        eprintln!(
            "[ipc_worker] Spawning child with py_bin={py_bin:?} in repo_root={:?}",
            cmd.get_current_dir()
        );
        let mut child = cmd.spawn()?;
        eprintln!("[ipc_worker] Child spawned with PID={}", child.id());

        #[cfg(windows)]
        let job_handle = unsafe {
            let job = CreateJobObjectW(None, PCWSTR::null()).ok();
            if let Some(h) = job {
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
                let _ = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    size,
                );
                let _ = AssignProcessToJobObject(h, HANDLE(child.as_raw_handle() as _));
                eprintln!("[ipc_worker] Child assigned to Win32 Job Object");
                Some(h)
            } else {
                None
            }
        };

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stdin_holder = Arc::new(Mutex::new(stdin));
        let child_holder = Arc::new(Mutex::new(Some(child)));

        // Reader thread: consumes JSON lines from child.stdout
        if let Some(out) = stdout {
            let reader_sink = sink.clone();
            thread::Builder::new()
                .name("nopeek-ipc-reader".into())
                .spawn(move || {
                    let reader = BufReader::new(out);
                    for line_res in reader.lines() {
                        match line_res {
                            Ok(line) => {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    continue;
                                }
                                match IpcEvent::parse_line(trimmed) {
                                    Ok(event) => {
                                        eprintln!("[ipc_reader] Received event: {event:?}");
                                        reader_sink.send(AppEvent::Ipc(event));
                                    }
                                    Err(e) => {
                                        eprintln!("[nopeek-gui] failed to parse JSON line: {e} | line: {trimmed}");
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                })?;
        }

        // Monitor thread: awaits child process termination
        let monitor_child = Arc::clone(&child_holder);
        let monitor_sink = sink;
        thread::Builder::new()
            .name("nopeek-ipc-monitor".into())
            .spawn(move || {
                let code = loop {
                    thread::sleep(std::time::Duration::from_millis(250));
                    let mut lock = monitor_child.lock().unwrap();
                    if let Some(c) = lock.as_mut() {
                        match c.try_wait() {
                            Ok(Some(status)) => break status.code(),
                            Ok(None) => {}
                            Err(_) => break None,
                        }
                    } else {
                        break None;
                    }
                };
                monitor_sink.send(AppEvent::IpcExited { code });
            })?;

        Ok(Self {
            stdin: stdin_holder,
            #[cfg(windows)]
            job_handle,
            child_handle: child_holder,
        })
    }

    /// Send an IPC command to the Python worker via stdin.
    pub fn send_command(&self, cmd: &IpcCommand) -> std::io::Result<()> {
        let json = cmd.to_jsonl();
        let mut lock = self.stdin.lock().unwrap();
        if let Some(stdin) = lock.as_mut() {
            stdin.write_all(json.as_bytes())?;
            stdin.flush()?;
        }
        Ok(())
    }
}

impl Drop for IpcWorker {
    fn drop(&mut self) {
        // Send quit command to child
        let _ = self.send_command(&IpcCommand::Quit);

        #[cfg(windows)]
        if let Some(job) = self.job_handle.take() {
            unsafe {
                let _ = CloseHandle(job);
            }
        }

        // Ensure child process terminates
        if let Ok(mut lock) = self.child_handle.lock() {
            if let Some(mut c) = lock.take() {
                let _ = c.kill();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_python_locates_existing_interpreter() {
        let py = IpcWorker::find_python();
        assert!(py.exists(), "Python interpreter path must exist: {:?}", py);
    }
}
