//! Private local payloads grant access to their owner and OS administrators
//! only. Never change an existing imported source's access-control list.
use std::{
    fs::{File, OpenOptions},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr::NonNull,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{Authorization::*, *},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn sid_text(sid: PSID) -> Result<String, String> {
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let _owned = Local(text.cast());
    let mut len = 0;
    unsafe {
        while *text.add(len) != 0 && len < 256 {
            len += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            text, len,
        )))
    }
}
pub(crate) fn current_sid() -> Result<String, String> {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    struct Token(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let _owned = Token(token);
    let mut size = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 65536 {
        return Err("利用者の所有情報を取得できません".into());
    }
    let mut data = vec![0u64; (size as usize).div_ceil(8)];
    if unsafe { GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size) }
        == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    sid_text(unsafe { (*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid })
}
pub(super) fn current_owner() -> Result<u32, String> {
    Ok(current_sid()?.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    }))
}
pub(super) fn metadata(file: &File) -> Result<(u32, bool), String> {
    let mut owner = std::ptr::null_mut();
    let mut dacl = std::ptr::null_mut();
    let mut sd = std::ptr::null_mut();
    let error = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if error != 0 {
        return Err(std::io::Error::from_raw_os_error(error as i32).to_string());
    }
    let _owned = Local(sd);
    let owner = NonNull::new(owner).ok_or("ファイルの所有者情報がありません")?;
    let owner_text = sid_text(owner.as_ptr())?;
    let owner_id = owner_text.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    let Some(dacl) = NonNull::new(dacl) else {
        return Ok((owner_id, false));
    };
    // uid() is a compact compatibility identifier, not an authorization
    // boundary. Private ownership must compare the complete token SID so a
    // collision in that identifier cannot authorize another account's files.
    let mut private = owner_text == current_sid()?;
    // GetSecurityInfo outputs borrow from sd, retained by _owned until return.
    for index in 0..unsafe { dacl.as_ref().AceCount } {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(dacl.as_ptr(), u32::from(index), &mut ace) } == 0 {
            return Err("ファイル権限を検査できません".into());
        }
        let ace = NonNull::new(ace).ok_or("ファイル権限の項目がありません")?;
        // GetAce success supplies an entry inside the same live ACL allocation.
        let header = unsafe { ace.cast::<ACE_HEADER>().as_ref() };
        if header.AceFlags & 0x08 != 0 {
            continue;
        }
        // WinNT.h: ACCESS_ALLOWED_ACE_TYPE=0, ACCESS_DENIED_ACE_TYPE=1.
        if header.AceType == 1 {
            continue;
        }
        if header.AceType != 0 || header.AceSize < 12 {
            private = false;
            continue;
        }
        let allowed = unsafe { ace.cast::<ACCESS_ALLOWED_ACE>().as_ref() };
        if allowed.Mask == 0 {
            continue;
        }
        let sid = sid_text((&allowed.SidStart as *const u32).cast_mut().cast())?;
        if sid != owner_text && sid != "S-1-5-18" && sid != "S-1-5-32-544" {
            private = false;
        }
    }
    Ok((owner_id, private))
}
pub(crate) fn set_private(path: &Path) -> Result<(), String> {
    let _parents = super::parents(path.parent().ok_or("非公開領域の親がありません")?)?;
    let sid = current_sid()?;
    let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut sd = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut sd,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let _owned = Local(sd);
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = std::ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) } == 0
        || present == 0
        || dacl.is_null()
    {
        return Err("非公開権限の生成に失敗しました".into());
    }
    let file = OpenOptions::new()
        .access_mode(0x40000 | 0x20000 | 0x80)
        .share_mode(3)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    super::identity(&file)?;
    // Reapplying an identical inheritable DACL changes NTFS change times on
    // descendants. Cold recovery must retain their saved identity proofs.
    if metadata(&file)? == (current_owner()?, true) {
        return Ok(());
    }
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null(),
        )
    };
    if result != 0 {
        return Err(std::io::Error::from_raw_os_error(result as i32).to_string());
    }
    if !metadata(&file)?.1 {
        return Err("非公開権限を確認できません".into());
    }
    Ok(())
}
