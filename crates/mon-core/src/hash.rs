//! SHA-256 hex encoding shared across desktop and native crates.
use sha2::{Digest, Sha256};

pub fn sha256_hex(data: impl AsRef<[u8]>) -> String {
    bytes_hex(Sha256::digest(data).as_ref())
}

pub fn sha256_output_hex(output: impl AsRef<[u8]>) -> String {
    bytes_hex(output.as_ref())
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}