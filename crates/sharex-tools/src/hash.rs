use md5::{Digest as _, Md5};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use sharex_core::CoreError;
use std::fmt::{self, Write as _};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HashAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlgorithm {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha512 => "SHA-512",
        }
    }
}

impl fmt::Display for HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

pub fn compute_bytes_hash(alg: HashAlgorithm, bytes: &[u8]) -> String {
    match alg {
        HashAlgorithm::Md5 => encode_hex_lower(&Md5::digest(bytes)),
        HashAlgorithm::Sha1 => encode_hex_lower(&Sha1::digest(bytes)),
        HashAlgorithm::Sha256 => encode_hex_lower(&Sha256::digest(bytes)),
        HashAlgorithm::Sha512 => encode_hex_lower(&Sha512::digest(bytes)),
    }
}

pub fn compute_file_hash(alg: HashAlgorithm, path: &Path) -> Result<String, CoreError> {
    let mut file = File::open(path)?;
    match alg {
        HashAlgorithm::Md5 => hash_reader::<Md5>(&mut file),
        HashAlgorithm::Sha1 => hash_reader::<Sha1>(&mut file),
        HashAlgorithm::Sha256 => hash_reader::<Sha256>(&mut file),
        HashAlgorithm::Sha512 => hash_reader::<Sha512>(&mut file),
    }
}

pub fn verify_hash(computed: &str, expected: &str) -> bool {
    computed.trim().eq_ignore_ascii_case(expected.trim())
}

fn hash_reader<D: md5::Digest>(reader: &mut impl Read) -> Result<String, CoreError> {
    let mut hasher = D::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(encode_hex_lower(&hasher.finalize()))
}

fn encode_hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn matches_standard_rfc_and_nist_test_vectors() {
        let input = b"abc";
        assert_eq!(
            compute_bytes_hash(HashAlgorithm::Md5, input),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            compute_bytes_hash(HashAlgorithm::Sha1, input),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            compute_bytes_hash(HashAlgorithm::Sha256, input),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            compute_bytes_hash(HashAlgorithm::Sha512, input),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );

        assert_eq!(
            compute_bytes_hash(HashAlgorithm::Sha256, b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hashes_files_on_disk_and_verifies_case_insensitively() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("payload.bin");
        let payload = b"sharex-in-rust".repeat(8192);
        fs::write(&file_path, &payload).expect("write payload");

        for alg in [
            HashAlgorithm::Md5,
            HashAlgorithm::Sha1,
            HashAlgorithm::Sha256,
            HashAlgorithm::Sha512,
        ] {
            let from_bytes = compute_bytes_hash(alg, &payload);
            let from_file = compute_file_hash(alg, &file_path).expect("hash file");
            assert_eq!(from_file, from_bytes);
            assert!(verify_hash(
                &from_file,
                &format!("  {} \n", from_bytes.to_uppercase())
            ));
            assert!(!verify_hash(&from_file, "0000"));
        }

        let missing = dir.path().join("does-not-exist.bin");
        assert!(matches!(
            compute_file_hash(HashAlgorithm::Sha256, &missing),
            Err(CoreError::Io(_))
        ));
    }
}
