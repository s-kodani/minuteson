//! Every child starts suspended, joins its owned job, then becomes runnable.
//! The process-wide parent job also covers a crash between spawn and assignment.
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Child;
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessIdOfThread, OpenThread, ResumeThread,
    THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
};

pub(crate) struct Job(OwnedHandle);
static APPLICATION_JOB: OnceLock<Job> = OnceLock::new();
static INITIALIZATION: Mutex<()> = Mutex::new(());

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // Each successful Win32 constructor transfers one exclusively owned handle.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}

impl Job {
    pub(crate) fn new() -> io::Result<Self> {
        let handle = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    pub(crate) fn assign_and_resume(&self, child: &Child) -> io::Result<()> {
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let snapshot = owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
        let mut entry: THREADENTRY32 = unsafe { zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut present = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;
        let mut thread = None;
        while present {
            if entry.th32OwnerProcessID == child.id() {
                if thread.is_some() {
                    return Err(io::Error::other("suspended worker has multiple threads"));
                }
                let handle = owned(unsafe {
                    OpenThread(
                        THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                        0,
                        entry.th32ThreadID,
                    )
                })?;
                if unsafe { GetProcessIdOfThread(handle.as_raw_handle()) } != child.id() {
                    return Err(io::Error::other("worker thread ownership changed"));
                }
                thread = Some(handle);
            }
            present = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }
        let thread = thread.ok_or_else(|| io::Error::other("suspended worker has no thread"))?;
        if unsafe { ResumeThread(thread.as_raw_handle()) } != 1 {
            return Err(io::Error::other(
                "worker thread did not resume from its initial suspension",
            ));
        }
        Ok(())
    }
}

pub(crate) fn initialize() -> io::Result<()> {
    let _guard = INITIALIZATION
        .lock()
        .map_err(|_| io::Error::other("application job initialization poisoned"))?;
    if APPLICATION_JOB.get().is_some() {
        return Ok(());
    }
    let job = Job::new()?;
    // Static ownership deliberately lasts until OS process teardown. Dropping
    // this handle while MON is alive would terminate MON as well as its children.
    if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), GetCurrentProcess()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // Only the startup thread initializes this before any worker is spawned.
    APPLICATION_JOB
        .set(job)
        .map_err(|_| io::Error::other("application job already initialized"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_engine::{ManagedChild, OwnedCommand};
    use std::os::windows::io::AsHandle;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, CREATE_NO_WINDOW, PROCESS_SYNCHRONIZE,
    };

    fn fixture_command(directory: &Path, mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "local_engine::windows::tests::process_fixture",
                "--ignored",
            ])
            .env("MON_WINDOWS_JOB_FIXTURE", mode)
            .env("MON_WINDOWS_JOB_DIRECTORY", directory)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn wait_for(path: &Path) {
        let start = Instant::now();
        while !path.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "fixture did not publish {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn descendant(directory: &Path) -> OwnedHandle {
        let path = directory.join("descendant.pid");
        wait_for(&path);
        let pid = std::fs::read_to_string(path)
            .unwrap()
            .parse::<u32>()
            .unwrap();
        let handle = owned(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) }).unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        handle
    }

    #[test]
    #[ignore = "Child-process fixture, invoked only with a private directory and mode"]
    fn process_fixture() {
        let mode = std::env::var("MON_WINDOWS_JOB_FIXTURE").expect("missing fixture mode");
        let directory = std::env::var_os("MON_WINDOWS_JOB_DIRECTORY").unwrap();
        let directory = Path::new(&directory);
        match mode.as_str() {
            "descendant" => {
                std::fs::write(
                    directory.join("descendant.pid"),
                    std::process::id().to_string(),
                )
                .unwrap();
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            "worker" => {
                // This ordinary child must inherit the job; it does not use MON's launcher.
                let mut child = fixture_command(directory, "descendant").spawn().unwrap();
                let _ = child.wait();
            }
            "owner" => {
                initialize().unwrap();
                let mut command = OwnedCommand::new(&std::env::current_exe().unwrap()).unwrap();
                command.command = fixture_command(directory, "worker");
                let _worker = command.spawn().unwrap();
                wait_for(&directory.join("exit-owner"));
                // Deliberately bypass Drop to exercise OS handle teardown.
                std::process::exit(0);
            }
            _ => panic!("unknown fixture mode"),
        }
    }

    #[test]
    fn owned_job_closes_worker_and_ordinary_descendants_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let mut command = OwnedCommand::new(&std::env::current_exe().unwrap()).unwrap();
        command.command = fixture_command(directory.path(), "worker");
        let child = command.spawn().unwrap();
        let process = child.child.as_handle().try_clone_to_owned().unwrap();
        let leaf = descendant(directory.path());
        drop(child);
        for handle in [process, leaf] {
            assert_eq!(
                unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
                WAIT_OBJECT_0
            );
        }
        assert!(command.spawn().is_err());
    }

    #[test]
    fn parent_process_exit_without_drop_terminates_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let mut owner =
            ManagedChild::direct(fixture_command(directory.path(), "owner").spawn().unwrap());
        let leaf = descendant(directory.path());
        std::fs::write(directory.path().join("exit-owner"), b"exit").unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(owner.child.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
        assert!(owner.child.wait().unwrap().success());
        assert_eq!(
            unsafe { WaitForSingleObject(leaf.as_raw_handle(), 5000) },
            WAIT_OBJECT_0
        );
    }
}
