//! Release tooling: verify the exact Tauri package and its signed version before publication.
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use std::io::Read;

fn verify(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() != 5 {
        return Err(
            "usage: mon-update-verify PUBLIC_KEY_FILE SIGNATURE_FILE ARCHIVE VERSION".into(),
        );
    }
    let decode = |path: &str| -> Result<String, Box<dyn std::error::Error>> {
        Ok(String::from_utf8(
            STANDARD.decode(std::fs::read_to_string(path)?.trim())?,
        )?)
    };
    let key = PublicKey::decode(&decode(&args[1])?)?;
    let signature = Signature::decode(&decode(&args[2])?)?;
    let mut verifier = key.verify_stream(&signature)?;
    let mut file = std::fs::File::open(&args[3])?;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count]);
    }
    verifier.finalize()?;
    let signed_version = signature
        .trusted_comment()
        .split('\t')
        .find_map(|s| s.strip_prefix("version:"));
    if signed_version != Some(args[4].as_str()) {
        return Err("signed version differs from release version".into());
    }
    Ok(())
}
fn main() {
    if let Err(error) = verify(&std::env::args().collect::<Vec<_>>()) {
        eprintln!("Update verification failed: {error}");
        std::process::exit(1);
    }
}
