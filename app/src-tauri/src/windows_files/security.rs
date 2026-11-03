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
    Storage::FileSystem::FILE_ALL_ACCESS,
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
    let (owner, private) = inspect(file)?;
    Ok((
        owner.bytes().fold(2166136261u32, |h, b| {
            (h ^ u32::from(b)).wrapping_mul(16777619)
        }),
        private,
    ))
}
fn inspect(file: &File) -> Result<(String, bool), String> {
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
    let Some(dacl) = NonNull::new(dacl) else {
        return Ok((owner_text, false));
    };
    // uid() is a compatibility identifier. Authorize against the complete SID.
    // Elevated tokens may give inherited children an Administrators owner.
    // Such an object belongs logically to this user only when its effective
    // DACL grants this exact user full control and excludes other users.
    let user = current_sid()?;
    let administrator_owner = owner_text == "S-1-5-32-544";
    let mut private = owner_text == user || administrator_owner;
    let mut user_controls = false;
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
        if sid == user && allowed.Mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS {
            user_controls = true;
        }
        if sid != user && sid != "S-1-5-18" && sid != "S-1-5-32-544" {
            private = false;
        }
    }
    if administrator_owner {
        private &= user_controls;
        if private {
            return Ok((user, true));
        }
    }
    Ok((owner_text, private))
}
pub(crate) fn set_private(path: &Path) -> Result<(), String> {
    let _parents = super::parents(path.parent().ok_or("非公開領域の親がありません")?)?;
    let sid = current_sid()?;
    let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
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
    let mut owner = std::ptr::null_mut();
    let mut owner_defaulted = 0;
    if unsafe { GetSecurityDescriptorOwner(sd, &mut owner, &mut owner_defaulted) } == 0
        || owner.is_null()
    {
        return Err("非公開領域の所有者を生成できません".into());
    }
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
    let (existing_owner, private) = inspect(&file)?;
    if existing_owner == sid && private {
        return Ok(());
    }
    // Elevated Windows tokens can default new objects to Administrators.
    // Bind our private payload to the actual user; never adopt another user's
    // object. Keep the original handle/parents pinned while requesting the
    // additional WRITE_OWNER right only for this group-owned case.
    let change_owner = existing_owner != sid;
    let file = if change_owner {
        if existing_owner != "S-1-5-32-544" {
            return Err("非公開領域の所有者が現在の利用者と異なります".into());
        }
        let owned = OpenOptions::new()
            .access_mode(0x80000 | 0x40000 | 0x20000 | 0x80)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)
            .map_err(|e| e.to_string())?;
        if super::identity(&owned)? != super::identity(&file)? {
            return Err("非公開領域の実体が変更されています".into());
        }
        owned
    } else {
        file
    };
    let result = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION
                | if change_owner {
                    OWNER_SECURITY_INFORMATION
                } else {
                    0
                },
            if change_owner {
                owner
            } else {
                std::ptr::null_mut()
            },
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
