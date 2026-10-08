//! Minimal dynamically loaded C ABI from the pinned sherpa-onnx 1.13.8 header.
//! Runs only in the task-owned headless child, never on a Tauri/UI thread.
use super::{wave, Output, Request, MAX_DIMENSION, VERSION};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}
struct Library(*mut c_void);
impl Library {
    fn open(path: &Path) -> Result<Self, String> {
        let path = CString::new(path.to_str().ok_or("invalid library path")?)
            .map_err(|e| e.to_string())?;
        // Darwin RTLD_NOW | RTLD_LOCAL; resolve required symbols before inference.
        let pointer = unsafe { dlopen(path.as_ptr(), 2 | 4) };
        if pointer.is_null() {
            Err("could not load fixed speaker runtime".into())
        } else {
            Ok(Self(pointer))
        }
    }
    fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        let symbol = unsafe { dlsym(self.0, name.as_ptr()) };
        if symbol.is_null() {
            Err("fixed speaker runtime is missing a required symbol".into())
        } else {
            Ok(symbol)
        }
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            dlclose(self.0);
        }
    }
}

#[repr(C)]
struct Config {
    model: *const c_char,
    threads: i32,
    debug: i32,
    provider: *const c_char,
}
type Handle = *const c_void;
struct Api<'a> {
    _library: &'a Library,
    create: unsafe extern "C" fn(*const Config) -> Handle,
    destroy: unsafe extern "C" fn(Handle),
    dimension: unsafe extern "C" fn(Handle) -> i32,
    stream: unsafe extern "C" fn(Handle) -> Handle,
    destroy_stream: unsafe extern "C" fn(Handle),
    accept: unsafe extern "C" fn(Handle, i32, *const f32, i32),
    finish: unsafe extern "C" fn(Handle),
    ready: unsafe extern "C" fn(Handle, Handle) -> i32,
    compute: unsafe extern "C" fn(Handle, Handle) -> *const f32,
    destroy_embedding: unsafe extern "C" fn(*const f32),
}
impl<'a> Api<'a> {
    fn load(library: &'a Library) -> Result<Self, String> {
        // Each transmute has an explicit signature copied from the public C
        // declarations, NOT from an inferred C++ object layout.
        let version: unsafe extern "C" fn() -> *const c_char =
            unsafe { std::mem::transmute(library.symbol("SherpaOnnxGetVersionStr")?) };
        let pointer = unsafe { version() };
        if pointer.is_null() {
            return Err("speaker runtime has no version".into());
        }
        if unsafe { CStr::from_ptr(pointer) }
            .to_str()
            .map_err(|e| e.to_string())?
            != VERSION
        {
            return Err("unsupported speaker runtime ABI version".into());
        }
        Ok(unsafe {
            Self {
                _library: library,
                create: std::mem::transmute(
                    library.symbol("SherpaOnnxCreateSpeakerEmbeddingExtractor")?,
                ),
                destroy: std::mem::transmute(
                    library.symbol("SherpaOnnxDestroySpeakerEmbeddingExtractor")?,
                ),
                dimension: std::mem::transmute(
                    library.symbol("SherpaOnnxSpeakerEmbeddingExtractorDim")?,
                ),
                stream: std::mem::transmute(
                    library.symbol("SherpaOnnxSpeakerEmbeddingExtractorCreateStream")?,
                ),
                destroy_stream: std::mem::transmute(
                    library.symbol("SherpaOnnxDestroyOnlineStream")?,
                ),
                accept: std::mem::transmute(
                    library.symbol("SherpaOnnxOnlineStreamAcceptWaveform")?,
                ),
                finish: std::mem::transmute(library.symbol("SherpaOnnxOnlineStreamInputFinished")?),
                ready: std::mem::transmute(
                    library.symbol("SherpaOnnxSpeakerEmbeddingExtractorIsReady")?,
                ),
                compute: std::mem::transmute(
                    library.symbol("SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding")?,
                ),
                destroy_embedding: std::mem::transmute(
                    library.symbol("SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding")?,
                ),
            }
        })
    }
}
struct Extractor<'a>(&'a Api<'a>, Handle);
impl Drop for Extractor<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.0.destroy)(self.1);
        }
    }
}
struct Stream<'a>(&'a Extractor<'a>, Handle);
impl Drop for Stream<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.0 .0.destroy_stream)(self.1);
        }
    }
}
struct Embedding<'a>(&'a Stream<'a>, *const f32);
impl Drop for Embedding<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.0 .0 .0.destroy_embedding)(self.1);
        }
    }
}

pub(super) fn extract(request: &Request, samples: &[f32]) -> Result<Output, String> {
    if samples.is_empty()
        || samples.len() > 480_000
        || request.end.checked_sub(request.first) != Some(samples.len() as u64)
        || !(1..=64).contains(&request.threads)
        || samples
            .iter()
            .any(|value| !value.is_finite() || !(-1.0..=1.0).contains(value))
    {
        return Err("invalid native feature input".into());
    }
    let library_identity = wave::identity(&request.library)?;
    let model_identity = wave::identity(&request.model)?;
    let output = {
        let library = Library::open(&request.library)?;
        let api = Api::load(&library)?;
        let model = CString::new(request.model.to_str().ok_or("invalid model path")?)
            .map_err(|e| e.to_string())?;
        let config = Config {
            model: model.as_ptr(),
            threads: request.threads,
            debug: 0,
            provider: c"cpu".as_ptr(),
        };
        let pointer = unsafe { (api.create)(&config) };
        if pointer.is_null() {
            return Err("speaker model initialization failed".into());
        }
        let extractor = Extractor(&api, pointer);
        let dimension = unsafe { (api.dimension)(extractor.1) };
        if !(1..=MAX_DIMENSION as i32).contains(&dimension) {
            return Err("unsupported embedding dimension".into());
        }
        let pointer = unsafe { (api.stream)(extractor.1) };
        if pointer.is_null() {
            return Err("speaker stream initialization failed".into());
        }
        let stream = Stream(&extractor, pointer);
        unsafe {
            (api.accept)(stream.1, 16_000, samples.as_ptr(), samples.len() as i32);
            (api.finish)(stream.1);
        }
        match unsafe { (api.ready)(extractor.1, stream.1) } {
            0 => Output {
                dimension: dimension as usize,
                vector: None,
                reason: Some("insufficient_audio"),
            },
            1 => {
                let pointer = unsafe { (api.compute)(extractor.1, stream.1) };
                if pointer.is_null() {
                    return Err("speaker embedding extraction failed".into());
                }
                let embedding = Embedding(&stream, pointer);
                let vector =
                    unsafe { std::slice::from_raw_parts(embedding.1, dimension as usize) }.to_vec();
                let norm = vector
                    .iter()
                    .map(|value| f64::from(*value).powi(2))
                    .sum::<f64>()
                    .sqrt();
                let reason = if vector.iter().any(|value| !value.is_finite()) {
                    Some("non_finite_embedding")
                } else if !norm.is_finite() || norm <= 1e-12 {
                    Some("zero_embedding")
                } else {
                    None
                };
                Output {
                    dimension: dimension as usize,
                    vector: reason.is_none().then_some(vector),
                    reason,
                }
            }
            _ => return Err("invalid speaker readiness result".into()),
        }
    };
    wave::same(&library_identity, &wave::identity(&request.library)?)?;
    wave::same(&model_identity, &wave::identity(&request.model)?)?;
    Ok(output)
}
