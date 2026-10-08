//! Headless, bounded speaker-feature extraction. No capture, network or DB.
//! The parent must verify helper/model/runtime/WAV fingerprints before and
//! after execution, own/reap this process, and enforce its inference timeout.
#[cfg(target_os = "macos")]
mod native;
mod wave;
use std::path::PathBuf;

pub const PROTOCOL: u32 = 1;
pub const VERSION: &str = "1.13.8";
pub const MAX_DIMENSION: usize = 4096;

pub struct Request {
    pub library: PathBuf,
    pub model: PathBuf,
    pub wave: PathBuf,
    pub first: u64,
    pub end: u64,
    pub threads: i32,
}

fn number(value: &str) -> Result<u64, String> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid numeric argument".into());
    }
    let result: u64 = value.parse().map_err(|_| "numeric argument overflow")?;
    if result.to_string() != value {
        return Err("noncanonical numeric argument".into());
    }
    Ok(result)
}

fn request(args: &[String]) -> Result<Request, String> {
    if args.len() != 14 {
        return Err("seven named argument pairs are required".into());
    }
    let mut fields = std::collections::BTreeMap::new();
    for pair in args.chunks_exact(2) {
        if fields.insert(pair[0].as_str(), pair[1].as_str()).is_some() {
            return Err("duplicate argument".into());
        }
    }
    let mut take = |name| fields.remove(name).ok_or_else(|| format!("missing {name}"));
    if number(take("--protocol-version")?)? != u64::from(PROTOCOL) {
        return Err("unsupported protocol".into());
    }
    let library = PathBuf::from(take("--library")?);
    let model = PathBuf::from(take("--model")?);
    let wave = PathBuf::from(take("--wave")?);
    let first = number(take("--first-frame")?)?;
    let end = number(take("--end-frame")?)?;
    let threads = number(take("--threads")?)?;
    if !fields.is_empty()
        || !(1..=64).contains(&threads)
        || first >= end
        || end - first > 30 * 16_000
        || [&library, &model, &wave].iter().any(|path| {
            !path.is_absolute()
                || path
                    .to_str()
                    .is_none_or(|value| value.contains(['\0', '\n', '\r']) || value.len() > 4096)
        })
    {
        return Err("invalid paths, threads or selected window".into());
    }
    Ok(Request {
        library,
        model,
        wave,
        first,
        end,
        threads: threads as i32,
    })
}

pub struct Output {
    pub dimension: usize,
    pub vector: Option<Vec<f32>>,
    pub reason: Option<&'static str>,
}
impl Output {
    pub fn json(&self, request: &Request) -> Result<String, String> {
        if !(1..=MAX_DIMENSION).contains(&self.dimension)
            || self.vector.as_ref().is_some_and(|vector| {
                vector.len() != self.dimension || vector.iter().any(|value| !value.is_finite())
            })
            || self.vector.is_some() != self.reason.is_none()
            || self.reason.is_some_and(|reason| {
                !matches!(
                    reason,
                    "insufficient_audio" | "non_finite_embedding" | "zero_embedding"
                )
            })
        {
            return Err("invalid native output".into());
        }
        let vector = self.vector.as_ref().map_or_else(
            || "null".into(),
            |vector| {
                format!(
                    "[{}]",
                    vector
                        .iter()
                        .map(|value| format!("\"{:08x}\"", value.to_bits()))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            },
        );
        let reason = self
            .reason
            .map_or_else(|| "null".into(), |reason| format!("\"{reason}\""));
        Ok(format!("{{\"schemaVersion\":{PROTOCOL},\"engine\":\"sherpa-onnx\",\"engineVersion\":\"{VERSION}\",\"sampleRate\":16000,\"firstFrame\":{},\"endFrame\":{},\"dimension\":{},\"status\":\"{}\",\"reason\":{reason},\"vectorBits\":{vector}}}",request.first,request.end,self.dimension,if self.vector.is_some(){"ready"}else{"unavailable"}))
    }
}

fn execute(args: &[String]) -> Result<String, String> {
    let request = request(args)?;
    let samples = wave::selected(&request.wave, request.first, request.end)?;
    #[cfg(target_os = "macos")]
    let output = native::extract(&request, &samples)?;
    #[cfg(not(target_os = "macos"))]
    let output: Output = {
        let _ = samples;
        return Err("this helper currently supports macOS only".into());
    };
    output.json(&request)
}

fn main() {
    match execute(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("speaker embedding failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests;
