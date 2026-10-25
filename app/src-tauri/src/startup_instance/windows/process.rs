use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::Path,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::ConvertSidToStringSidW, *},
    System::{RestartManager::*, Threading::*},
    UI::WindowsAndMessaging::*,
};
pub(super) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn owned(handle: HANDLE) -> Result<OwnedHandle, String> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }
}
fn timestamp(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}
fn created(handle: HANDLE) -> Result<u64, String> {
    let (mut start, mut end, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    if unsafe { GetProcessTimes(handle, &mut start, &mut end, &mut kernel, &mut user) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(timestamp(start))
}
pub(super) fn current_identity() -> Result<(u32, u64), String> {
    Ok((std::process::id(), created(unsafe { GetCurrentProcess() })?))
}
fn sid(handle: HANDLE) -> Result<String, String> {
    let mut raw = std::ptr::null_mut();
    if unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut raw) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let token = owned(raw)?;
    let mut size = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut size,
        );
    }
    if size == 0 || size > 65536 {
        return Err("プロセスの所有者を確認できません".into());
    }
    let mut bytes = vec![0u64; (size as usize).div_ceil(8)];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            bytes.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut text = std::ptr::null_mut();
    if unsafe {
        ConvertSidToStringSidW((*(bytes.as_ptr().cast::<TOKEN_USER>())).User.Sid, &mut text)
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut len = 0;
    unsafe {
        while *text.add(len) != 0 && len < 256 {
            len += 1;
        }
    }
    let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    unsafe {
        LocalFree(text.cast());
    }
    Ok(result)
}
pub(super) struct Target {
    handle: OwnedHandle,
    pid: u32,
    created: u64,
}
impl Target {
    pub(super) fn wait_exit(&self, timeout: Duration) -> bool {
        unsafe {
            WaitForSingleObject(self.handle.as_raw_handle(), timeout.as_millis() as u32)
                == WAIT_OBJECT_0
        }
    }
    pub(super) fn open(pid: u32, executable: &Path) -> Result<Self, String> {
        if pid == std::process::id() {
            return Err("現在の復旧画面は終了対象にできません".into());
        }
        let handle = owned(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        })?;
        let mut path = vec![0u16; 32768];
        let mut len = path.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(handle.as_raw_handle(), 0, path.as_mut_ptr(), &mut len)
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let actual = std::path::PathBuf::from(String::from_utf16_lossy(&path[..len as usize]))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if actual.to_string_lossy().to_lowercase() != executable.to_string_lossy().to_lowercase()
            || sid(handle.as_raw_handle())? != crate::windows_files::current_sid()?
        {
            return Err("同じ実行ファイル・利用者のMONではないため操作しません".into());
        }
        let start = created(handle.as_raw_handle())?;
        Ok(Self {
            handle,
            pid,
            created: start,
        })
    }
    pub(super) fn created(&self) -> u64 {
        self.created
    }
    pub(super) fn identity(&self) -> (u32, u64) {
        (self.pid, self.created)
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
    pub(super) fn exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) == WAIT_OBJECT_0 }
    }
    pub(super) fn allow_foreground(&self) {
        unsafe {
            AllowSetForegroundWindow(self.pid);
        }
    }
    pub(super) fn terminate_checked(
        &self,
        executable: &Path,
        timeout: Duration,
    ) -> Result<(), String> {
        if self.exited() {
            return Ok(());
        }
        let current = Self::open(self.pid, executable)?;
        if !self.same(&current) {
            return Err("プロセスが入れ替わりました。終了していません。".into());
        }
        let stop = owned(unsafe {
            OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                self.pid,
            )
        })?;
        if created(stop.as_raw_handle())? != self.created {
            return Err("プロセスの生成時刻が変わりました。終了していません。".into());
        }
        if unsafe { TerminateProcess(stop.as_raw_handle(), 1) } == 0 && !self.exited() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if unsafe { WaitForSingleObject(self.handle.as_raw_handle(), timeout.as_millis() as u32) }
            != WAIT_OBJECT_0
        {
            return Err("MONの終了が未確認です。再起動していません。".into());
        }
        Ok(())
    }
    pub(super) fn restore_legacy(&self, title: &str) -> Result<bool, String> {
        struct Find {
            pid: u32,
            title: Vec<u16>,
            found: Vec<HWND>,
        }
        unsafe extern "system" fn visit(hwnd: HWND, value: LPARAM) -> i32 {
            let state = unsafe { &mut *(value as *mut Find) };
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(hwnd, &mut pid);
            }
            if pid != state.pid {
                return 1;
            }
            let mut text = vec![0u16; 1024];
            let len = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
            if text[..len as usize] == state.title {
                state.found.push(hwnd);
            }
            1
        }
        let mut state = Find {
            pid: self.pid,
            title: title.encode_utf16().collect(),
            found: Vec::new(),
        };
        unsafe {
            EnumWindows(Some(visit), &mut state as *mut _ as isize);
        }
        if state.found.len() != 1 {
            return Ok(false);
        }
        let hwnd = state.found[0];
        let mut result = 0;
        if unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_NULL,
                0,
                0,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                500,
                &mut result,
            )
        } == 0
        {
            return Ok(false);
        }
        self.allow_foreground();
        unsafe {
            ShowWindowAsync(hwnd, SW_RESTORE);
            ShowWindowAsync(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        loop {
            if self.exited() {
                return Ok(false);
            }
            if unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 } {
                return Ok(true);
            }
            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
pub(super) fn lock_owners(path: &Path) -> Result<Vec<(u32, u64)>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut session = 0;
    let mut key = [0u16; 33];
    let code = unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) };
    if code != 0 {
        return Err(std::io::Error::from_raw_os_error(code as i32).to_string());
    }
    struct Session(u32);
    impl Drop for Session {
        fn drop(&mut self) {
            unsafe {
                RmEndSession(self.0);
            }
        }
    }
    let _session = Session(session);
    let name = wide(&path.to_string_lossy());
    let ptr = name.as_ptr();
    let code =
        unsafe { RmRegisterResources(session, 1, &ptr, 0, std::ptr::null(), 0, std::ptr::null()) };
    if code != 0 {
        return Err(std::io::Error::from_raw_os_error(code as i32).to_string());
    }
    for _ in 0..3 {
        let (mut needed, mut count, mut reasons) = (0, 0, 0);
        let code = unsafe {
            RmGetList(
                session,
                &mut needed,
                &mut count,
                std::ptr::null_mut(),
                &mut reasons,
            )
        };
        if code == 0 {
            return Ok(Vec::new());
        }
        if code != ERROR_MORE_DATA || needed > 64 {
            return Err("起動設定を使用中のプロセスを限定できません".into());
        }
        let mut entries = vec![unsafe { std::mem::zeroed::<RM_PROCESS_INFO>() }; needed as usize];
        count = needed;
        let code = unsafe {
            RmGetList(
                session,
                &mut needed,
                &mut count,
                entries.as_mut_ptr(),
                &mut reasons,
            )
        };
        if code == ERROR_MORE_DATA {
            continue;
        }
        if code != 0 {
            return Err(std::io::Error::from_raw_os_error(code as i32).to_string());
        }
        return Ok(entries[..count as usize]
            .iter()
            .map(|e| (e.Process.dwProcessId, timestamp(e.Process.ProcessStartTime)))
            .collect());
    }
    Err("起動設定の所有者が変化しています。再確認してください。".into())
}
