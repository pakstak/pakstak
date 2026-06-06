use ring::digest;
use std::fmt::Write as _;
use std::io::{self, Read};
use thiserror::Error;

pub struct DigestVerifier<R> {
    reader: R,
    expected: Vec<u8>,
    context: digest::Context,
}

#[derive(Debug, Error)]
pub enum DigestError {
    #[error("digest `{0}` must use <algorithm>:<hex> format")]
    InvalidFormat(String),
    #[error("unsupported digest algorithm `{0}`")]
    UnsupportedAlgorithm(String),
    #[error("invalid digest `{digest}`: {reason}")]
    InvalidHex { digest: String, reason: String },
    #[error(transparent)]
    VerificationFailed(#[from] VerificationFailed),
}

#[derive(Debug, Error)]
#[error(
    "digest verification failed: expected {}:{}, got {}:{}",
    algorithm_name(.algorithm), encode_hex(.expected),
    algorithm_name(.algorithm), encode_hex(.actual)
)]
pub struct VerificationFailed {
    expected: Vec<u8>,
    actual: Vec<u8>,
    algorithm: &'static digest::Algorithm,
}

impl<R: Read> DigestVerifier<R> {
    pub fn new(reader: R, digest: &str) -> Result<Self, DigestError> {
        let (algorithm_name, hex) = digest
            .split_once(':')
            .ok_or_else(|| DigestError::InvalidFormat(digest.to_string()))?;

        let algorithm = algorithm(algorithm_name)
            .ok_or_else(|| DigestError::UnsupportedAlgorithm(algorithm_name.to_string()))?;

        let bytes = decode_hex(hex).map_err(|reason| DigestError::InvalidHex {
            digest: digest.to_string(),
            reason,
        })?;

        if bytes.len() != algorithm.output_len() {
            return Err(DigestError::InvalidHex {
                digest: digest.to_string(),
                reason: format!(
                    "expected {} bytes for {}, got {}",
                    algorithm.output_len(),
                    algorithm_name,
                    bytes.len()
                ),
            });
        }

        let context = digest::Context::new(algorithm);

        Ok(Self {
            reader,
            context,
            expected: bytes,
        })
    }

    pub fn verify(self) -> Result<(), VerificationFailed> {
        let actual = self.context.finish();
        if actual.as_ref() == self.expected {
            Ok(())
        } else {
            Err(VerificationFailed {
                expected: self.expected,
                actual: actual.as_ref().to_vec(),
                algorithm: actual.algorithm(),
            })
        }
    }
}

impl<R: Read> Read for DigestVerifier<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.reader.read(buffer)?;
        if read > 0 {
            self.context.update(&buffer[..read]);
        }
        Ok(read)
    }
}

pub fn verify_bytes(bytes: &[u8], expected_digest: &str) -> Result<(), DigestError> {
    let mut verifier = DigestVerifier::new(bytes, expected_digest)?;
    io::copy(&mut verifier, &mut io::sink()).map_err(|error| DigestError::InvalidHex {
        digest: expected_digest.to_string(),
        reason: format!("failed to hash bytes: {error}"),
    })?;
    verifier.verify().map_err(DigestError::from)
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    let digest = digest::digest(&digest::SHA256, bytes);
    format!("sha256:{}", encode_hex(digest.as_ref()))
}

fn algorithm(name: &str) -> Option<&'static digest::Algorithm> {
    match name {
        "sha256" => Some(&digest::SHA256),
        "sha384" => Some(&digest::SHA384),
        "sha512" => Some(&digest::SHA512),
        _ => None,
    }
}

fn algorithm_name(algorithm: &'static digest::Algorithm) -> &'static str {
    if std::ptr::eq(algorithm, &digest::SHA256) {
        "sha256"
    } else if std::ptr::eq(algorithm, &digest::SHA384) {
        "sha384"
    } else if std::ptr::eq(algorithm, &digest::SHA512) {
        "sha512"
    } else {
        "unknown"
    }
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("hex digest has an odd number of characters".to_string());
    }

    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let hex_byte = std::str::from_utf8(pair)
                .map_err(|error| format!("hex digest is not valid UTF-8: {error}"))?;
            u8::from_str_radix(hex_byte, 16)
                .map_err(|error| format!("invalid hex byte `{hex_byte}`: {error}"))
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}
