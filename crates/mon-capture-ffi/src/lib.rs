//! Versioned C ABI for shared capture decisions. Borrowed inputs are read only
//! for the duration of a call. No allocator ownership crosses this boundary.
use mon_core::capture::commit::RawPrefix;
use mon_core::capture::{limits, request, supervisor::InputLifecycle};
use mon_core::media::voice::VoiceConverter;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// PCM and timestamps are borrowed during the callback. Returning false makes
/// the converter terminal; native IO failures must never drop audio silently.
pub type VoiceWrite =
    Option<unsafe extern "C" fn(*mut std::ffi::c_void, *const i16, usize, i64, i64) -> bool>;

#[no_mangle]
pub extern "C" fn mon_voice_new(rate: u32, channels: u32) -> *mut VoiceConverter {
    catch_unwind(|| {
        VoiceConverter::new(rate, channels)
            .map(|v| Box::into_raw(Box::new(v)))
            .unwrap_or(std::ptr::null_mut())
    })
    .unwrap_or(std::ptr::null_mut())
}
#[no_mangle]
pub unsafe extern "C" fn mon_voice_free(value: *mut VoiceConverter) {
    if !value.is_null() {
        drop(Box::from_raw(value));
    }
}
#[no_mangle]
pub unsafe extern "C" fn mon_voice_push(
    value: *mut VoiceConverter,
    samples: *const f32,
    count: usize,
    start_ns: i64,
    end_ns: i64,
    context: *mut std::ffi::c_void,
    write: VoiceWrite,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let (Some(value), Some(write)) = (value.as_mut(), write) else {
            return false;
        };
        if samples.is_null() || count == 0 || count > mon_core::media::voice::MAX_INPUT_FRAMES * 8 {
            return false;
        }
        let result = value.push(
            std::slice::from_raw_parts(samples, count),
            start_ns,
            end_ns,
            &mut |pcm, start, end| {
                if write(context, pcm.as_ptr(), pcm.len(), start, end) {
                    Ok(())
                } else {
                    Err("voice PCM sink failed")
                }
            },
        );
        if let Err(error) = result {
            eprintln!("mon_voice_push: {error}; start={start_ns}; end={end_ns}; samples={count}");
        }
        result.is_ok()
    }))
    .unwrap_or(false)
}
#[no_mangle]
pub unsafe extern "C" fn mon_voice_finish(
    value: *mut VoiceConverter,
    context: *mut std::ffi::c_void,
    write: VoiceWrite,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let (Some(value), Some(write)) = (value.as_mut(), write) else {
            return false;
        };
        value
            .finish(&mut |pcm, start, end| {
                if write(context, pcm.as_ptr(), pcm.len(), start, end) {
                    Ok(())
                } else {
                    Err("voice PCM sink failed")
                }
            })
            .is_ok()
    }))
    .unwrap_or(false)
}
#[no_mangle]
pub unsafe extern "C" fn mon_voice_caf_header(
    frames: u64,
    output: *mut u8,
    capacity: usize,
) -> isize {
    if output.is_null() || capacity < 68 {
        return -1;
    }
    match mon_core::media::caf::voice_header(frames) {
        Ok(bytes) => {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, 68);
            68
        }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn mon_capture_abi_version() -> u32 {
    1
}
#[no_mangle]
pub extern "C" fn mon_capture_limit(key: u32) -> u64 {
    match key {
        1 => u64::from(limits::MAX_SEGMENTS),
        2 => limits::MAX_ENTRY_BYTES,
        3 => limits::MAX_METADATA_BYTES,
        4 => limits::TIP_RESERVE_BYTES,
        5 => limits::MAX_CLOCK_MAPS,
        6 => limits::MAX_PENDING_BYTES,
        7 => limits::MAX_PENDING_NANOSECONDS,
        8 => u64::from(limits::MAX_PENDING_PACKETS),
        9 => limits::ACQUISITION_TIMEOUT_NANOSECONDS,
        10 => limits::RETRY_NANOSECONDS,
        _ => 0,
    }
}
#[no_mangle]
pub extern "C" fn mon_capture_permits_commit(
    segments: u32,
    metadata: u64,
    maps: u64,
    next_bytes: u64,
    next_maps: u64,
) -> bool {
    limits::permits_commit(segments, metadata, maps, next_bytes, next_maps)
}
#[no_mangle]
pub extern "C" fn mon_capture_permits_pending(
    bytes: u64,
    nanoseconds: u64,
    packets: u32,
    next_bytes: u64,
    next_nanoseconds: u64,
) -> bool {
    limits::permits_pending(bytes, nanoseconds, packets, next_bytes, next_nanoseconds)
}
unsafe fn utf8<'a>(ptr: *const u8, len: usize, max: usize) -> Option<&'a str> {
    if ptr.is_null() || len > max {
        return None;
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).ok()
}
#[no_mangle]
pub unsafe extern "C" fn mon_capture_valid_owner(ptr: *const u8, len: usize) -> bool {
    utf8(ptr, len, 36).is_some_and(request::valid_owner)
}
#[no_mangle]
pub unsafe extern "C" fn mon_capture_valid_uid(ptr: *const u8, len: usize) -> bool {
    utf8(ptr, len, 1024).is_some_and(request::valid_uid)
}
#[no_mangle]
pub unsafe extern "C" fn mon_capture_valid_selection(
    ptr: *const u8,
    len: usize,
    owner: *const u8,
    owner_len: usize,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(text) = utf8(ptr, len, 4096) else {
            return false;
        };
        let Some(owner) = utf8(owner, owner_len, 36) else {
            return false;
        };
        request::MicrophoneSelectionRequest::parse(text.as_bytes(), owner).is_ok()
    }))
    .unwrap_or(false)
}
#[no_mangle]
pub unsafe extern "C" fn mon_capture_lifecycle(state: *mut InputLifecycle, action: u32) -> i32 {
    let Some(state) = state.as_mut() else {
        return -1;
    };
    match action {
        1 => {
            if state.begin() {
                1
            } else {
                -1
            }
        }
        2 => {
            if state.connect() {
                1
            } else {
                -1
            }
        }
        3 | 4 => state
            .interrupt(action == 4)
            .map(|first| i32::from(first))
            .unwrap_or(-1),
        5 => {
            state.stop();
            1
        }
        _ => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mon_capture_catalog_order(
    ptr: *const u8,
    len: usize,
    applications: bool,
    own: u32,
    parent: u32,
    output: *mut u32,
    capacity: usize,
) -> isize {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(text) = utf8(ptr, len, 512 * 1024) else {
            return -1;
        };
        let Ok(order) =
            mon_core::capture::catalog::order(text.as_bytes(), applications, own, parent)
        else {
            return -1;
        };
        if order.len() > capacity || (!order.is_empty() && output.is_null()) {
            return -1;
        }
        if !order.is_empty() {
            std::ptr::copy_nonoverlapping(order.as_ptr(), output, order.len());
        }
        order.len() as isize
    }))
    .unwrap_or(-1)
}

pub type ReadAt = Option<unsafe extern "C" fn(*mut std::ffi::c_void, u64, *mut u8, usize) -> isize>;
#[no_mangle]
pub unsafe extern "C" fn mon_media_validate_m4a(
    length: u64,
    context: *mut std::ffi::c_void,
    read: ReadAt,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(read) = read else {
            return false;
        };
        mon_core::media::m4a::validate(length, |offset, output| {
            if read(context, offset, output.as_mut_ptr(), output.len()) != output.len() as isize {
                return Err("M4A cannot be read");
            }
            Ok(())
        })
        .is_ok()
    }))
    .unwrap_or(false)
}

#[no_mangle]
pub unsafe extern "C" fn mon_capture_caf_header(
    rate: u32,
    channels: u32,
    frames: u64,
    output: *mut u8,
    capacity: usize,
) -> isize {
    if output.is_null() || capacity < 68 {
        return -1;
    }
    match mon_core::media::caf::header(rate, channels, frames) {
        Ok(bytes) => {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
            68
        }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn mon_capture_prefix_empty() -> RawPrefix {
    RawPrefix::default()
}
#[no_mangle]
pub unsafe extern "C" fn mon_capture_prefix_hash(
    prefix: *const RawPrefix,
    output: *mut u8,
    capacity: usize,
) -> bool {
    let Some(prefix) = prefix.as_ref() else {
        return false;
    };
    let Some(hash) = prefix.hash() else {
        return false;
    };
    if output.is_null() || capacity < 64 {
        return false;
    }
    std::ptr::copy_nonoverlapping(hash.as_ptr(), output, 64);
    true
}
#[no_mangle]
pub unsafe extern "C" fn mon_capture_prepare_commit(
    prefix: *const RawPrefix,
    receipt: *const u8,
    length: usize,
    next: *mut RawPrefix,
    tip: *mut u8,
    capacity: usize,
) -> isize {
    catch_unwind(AssertUnwindSafe(|| {
        let (Some(prefix), Some(next)) = (prefix.as_ref(), next.as_mut()) else {
            return -1;
        };
        let Some(receipt) = utf8(receipt, length, limits::MAX_ENTRY_BYTES as usize) else {
            return -1;
        };
        let Ok(plan) = prefix.prepare(receipt.as_bytes()) else {
            return -1;
        };
        if tip.is_null() || capacity < plan.tip.len() {
            return -1;
        }
        std::ptr::copy_nonoverlapping(plan.tip.as_ptr(), tip, plan.tip.len());
        *next = plan.next;
        plan.tip.len() as isize
    }))
    .unwrap_or(-1)
}
#[no_mangle]
pub unsafe extern "C" fn mon_capture_prefix_tip(
    prefix: *const RawPrefix,
    owner: *const u8,
    owner_len: usize,
    lane: *const u8,
    lane_len: usize,
    closed: bool,
    tip: *mut u8,
    capacity: usize,
) -> isize {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(prefix) = prefix.as_ref() else {
            return -1;
        };
        let (Some(owner), Some(lane)) = (utf8(owner, owner_len, 36), utf8(lane, lane_len, 10))
        else {
            return -1;
        };
        let Ok(bytes) = prefix.tip(owner, lane, closed) else {
            return -1;
        };
        if tip.is_null() || capacity < bytes.len() {
            return -1;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), tip, bytes.len());
        bytes.len() as isize
    }))
    .unwrap_or(-1)
}
