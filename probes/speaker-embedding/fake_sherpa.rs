//! Test-only C ABI library. No real models, capture or external input.
use std::ffi::{c_char, c_void, CStr};
use std::io::Write;
use std::path::PathBuf;
#[repr(C)]
pub struct Config {
    model: *const c_char,
    threads: i32,
    debug: i32,
    provider: *const c_char,
}
struct Engine {
    mode: String,
    model: PathBuf,
    log: PathBuf,
}
struct Stream {
    mode: String,
    model: PathBuf,
    log: PathBuf,
    samples: Vec<f32>,
}
#[repr(C)]
struct Vector {
    values: [f32; 3],
    log: PathBuf,
}
fn log(path: &PathBuf, event: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{event}").unwrap();
}
#[no_mangle]
pub extern "C" fn SherpaOnnxGetVersionStr() -> *const c_char {
    #[cfg(mon_fake_wrong_version)]
    return c"1.13.9".as_ptr();
    #[cfg(not(mon_fake_wrong_version))]
    c"1.13.8".as_ptr()
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxCreateSpeakerEmbeddingExtractor(
    config: *const Config,
) -> *const c_void {
    let config = &*config;
    assert!((1..=64).contains(&config.threads));
    assert_eq!(config.debug, 0);
    assert_eq!(CStr::from_ptr(config.provider), c"cpu");
    let model = PathBuf::from(CStr::from_ptr(config.model).to_str().unwrap());
    let mode = std::fs::read_to_string(&model).unwrap();
    let path = model.with_extension("log");
    log(&path, "create");
    if mode == "nullcreate" {
        return std::ptr::null();
    }
    Box::into_raw(Box::new(Engine {
        mode,
        model,
        log: path,
    })) as *const c_void
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxDestroySpeakerEmbeddingExtractor(pointer: *const c_void) {
    let engine = Box::from_raw(pointer as *mut Engine);
    log(&engine.log, "destroy_extractor");
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxSpeakerEmbeddingExtractorDim(pointer: *const c_void) -> i32 {
    if (&*(pointer as *const Engine)).mode == "dimension" {
        4097
    } else {
        3
    }
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxSpeakerEmbeddingExtractorCreateStream(
    pointer: *const c_void,
) -> *const c_void {
    let engine = &*(pointer as *const Engine);
    log(&engine.log, "stream");
    if engine.mode == "nullstream" {
        return std::ptr::null();
    }
    Box::into_raw(Box::new(Stream {
        mode: engine.mode.clone(),
        model: engine.model.clone(),
        log: engine.log.clone(),
        samples: Vec::new(),
    })) as *const c_void
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxDestroyOnlineStream(pointer: *const c_void) {
    let stream = Box::from_raw(pointer as *mut Stream);
    log(&stream.log, "destroy_stream");
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxOnlineStreamAcceptWaveform(
    pointer: *const c_void,
    rate: i32,
    samples: *const f32,
    count: i32,
) {
    assert_eq!(rate, 16000);
    assert!(count > 0 && count <= 480000);
    let stream = &mut *(pointer as *mut Stream);
    stream
        .samples
        .extend_from_slice(std::slice::from_raw_parts(samples, count as usize));
    log(&stream.log, "accept");
    if matches!(
        stream.mode.as_str(),
        "hang" | "crash" | "stdout_flood" | "stderr_flood"
    ) {
        log(&stream.log, &format!("pid:{}", std::process::id()));
        match stream.mode.as_str() {
            "crash" => std::process::exit(7),
            "stdout_flood" => print!("{}", "x".repeat(128 * 1024)),
            "stderr_flood" => eprint!("{}", "x".repeat(2 * 1024 * 1024)),
            _ => {}
        }
        // A single owned process, with no descendants, must be killed/reaped
        // by the caller on cancellation, timeout or output overflow.
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    if stream.mode == "tamper_wave" {
        std::fs::OpenOptions::new()
            .append(true)
            .open(stream.model.parent().unwrap().join("run.wav"))
            .unwrap()
            .write_all(b"xx")
            .unwrap();
    }
    if stream.mode == "tamper" {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&stream.model)
            .unwrap()
            .write_all(b"x")
            .unwrap();
    }
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxOnlineStreamInputFinished(pointer: *const c_void) {
    log(&(*(pointer as *const Stream)).log, "finish");
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxSpeakerEmbeddingExtractorIsReady(
    _: *const c_void,
    pointer: *const c_void,
) -> i32 {
    match (&*(pointer as *const Stream)).mode.as_str() {
        "notready" => 0,
        "badready" => 2,
        _ => 1,
    }
}
#[no_mangle]
#[cfg(not(mon_fake_missing_symbol))]
pub unsafe extern "C" fn SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding(
    _: *const c_void,
    pointer: *const c_void,
) -> *const f32 {
    let stream = &*(pointer as *const Stream);
    log(&stream.log, "compute");
    if stream.mode == "nullvector" {
        return std::ptr::null();
    }
    let values = match stream.mode.as_str() {
        "zero" => [0.0; 3],
        "nan" => [f32::NAN; 3],
        _ => [
            stream.samples[0],
            *stream.samples.last().unwrap(),
            stream.samples.len() as f32,
        ],
    };
    Box::into_raw(Box::new(Vector {
        values,
        log: stream.log.clone(),
    })) as *const f32
}
#[no_mangle]
pub unsafe extern "C" fn SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding(pointer: *const f32) {
    let vector = Box::from_raw(pointer as *mut Vector);
    log(&vector.log, "destroy_embedding");
}
