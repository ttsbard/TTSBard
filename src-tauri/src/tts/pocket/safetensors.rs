//! Minimal read-only safetensors reader.
//!
//! Pocket TTS voice states are stored as safetensors. The `safetensors` crate
//! is not otherwise required by the application, and pulling it in would add a
//! second dependency stack for a tiny format. This reader parses the JSON
//! header and copies only the requested tensor, so a 200 MB voice file is never
//! loaded whole.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SafeTensorMeta {
    pub dtype: String,
    pub shape: Vec<usize>,
    pub begin: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct SafeTensorsFile {
    file: File,
    data_start: u64,
    tensors: BTreeMap<String, SafeTensorMeta>,
}

/// Split tensor bytes into fixed-size chunks after the caller validates the
/// total length against the dtype size. `read_f32` rejects any length mismatch
/// before decoding, so a non-empty remainder here would be malformed input.
fn fixed_chunks<const N: usize>(bytes: &[u8]) -> impl Iterator<Item = &[u8; N]> {
    let (chunks, remainder) = bytes.as_chunks::<N>();
    debug_assert!(
        remainder.is_empty(),
        "tensor byte length must be validated before decoding"
    );
    chunks.iter()
}

impl SafeTensorsFile {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path)
            .map_err(|error| format!("failed to open safetensors '{}': {error}", path.display()))?;
        let file_len = file
            .metadata()
            .map_err(|error| format!("failed to stat safetensors '{}': {error}", path.display()))?
            .len();
        let mut header_len_bytes = [0u8; 8];
        file.read_exact(&mut header_len_bytes)
            .map_err(|error| format!("failed to read safetensors header length: {error}"))?;
        let header_len = u64::from_le_bytes(header_len_bytes);
        if header_len == 0 || header_len > 16 * 1024 * 1024 || header_len + 8 > file_len {
            return Err(format!(
                "safetensors header length {header_len} is invalid for '{}'",
                path.display()
            ));
        }
        let mut header = vec![0u8; header_len as usize];
        file.read_exact(&mut header)
            .map_err(|error| format!("failed to read safetensors header: {error}"))?;
        let parsed: serde_json::Value = serde_json::from_slice(&header)
            .map_err(|error| format!("failed to parse safetensors header: {error}"))?;
        let object = parsed
            .as_object()
            .ok_or_else(|| "safetensors header is not a JSON object".to_string())?;
        let data_start = 8 + header_len;
        let data_len = file_len - data_start;
        let mut tensors = BTreeMap::new();
        for (name, value) in object {
            if name == "__metadata__" {
                continue;
            }
            let dtype = value
                .get("dtype")
                .and_then(|value| value.as_str())
                .ok_or_else(|| format!("safetensors tensor '{name}' has no dtype"))?
                .to_string();
            let shape = value
                .get("shape")
                .and_then(|value| value.as_array())
                .ok_or_else(|| format!("safetensors tensor '{name}' has no shape"))?
                .iter()
                .map(|dim| {
                    dim.as_u64()
                        .map(|dim| dim as usize)
                        .ok_or_else(|| format!("safetensors tensor '{name}' has a bad shape"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let offsets = value
                .get("data_offsets")
                .and_then(|value| value.as_array())
                .filter(|offsets| offsets.len() == 2)
                .ok_or_else(|| format!("safetensors tensor '{name}' has no data_offsets"))?;
            let begin = offsets[0]
                .as_u64()
                .ok_or_else(|| format!("safetensors tensor '{name}' has a bad begin offset"))?
                as usize;
            let end = offsets[1]
                .as_u64()
                .ok_or_else(|| format!("safetensors tensor '{name}' has a bad end offset"))?
                as usize;
            if begin > end || end as u64 > data_len {
                return Err(format!(
                    "safetensors tensor '{name}' offset range {begin}..{end} is outside the file"
                ));
            }
            tensors.insert(
                name.clone(),
                SafeTensorMeta {
                    dtype,
                    shape,
                    begin,
                    end,
                },
            );
        }
        if tensors.is_empty() {
            return Err("safetensors file has no tensors".to_string());
        }
        Ok(Self {
            file,
            data_start,
            tensors,
        })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tensors.contains_key(name)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.tensors.keys().map(String::as_str)
    }

    fn read_bytes(&mut self, name: &str) -> Result<(SafeTensorMeta, Vec<u8>), String> {
        let meta = self
            .tensors
            .get(name)
            .cloned()
            .ok_or_else(|| format!("safetensors tensor '{name}' is missing"))?;
        let length = meta.end - meta.begin;
        let mut bytes = vec![0u8; length];
        self.file
            .seek(SeekFrom::Start(self.data_start + meta.begin as u64))
            .map_err(|error| format!("failed to seek safetensors tensor '{name}': {error}"))?;
        self.file
            .read_exact(&mut bytes)
            .map_err(|error| format!("failed to read safetensors tensor '{name}': {error}"))?;
        Ok((meta, bytes))
    }

    pub fn read_f32(&mut self, name: &str) -> Result<(Vec<usize>, Vec<f32>), String> {
        let (meta, bytes) = self.read_bytes(name)?;
        let count = meta.shape.iter().product::<usize>();
        let expected = match meta.dtype.as_str() {
            "F32" | "I32" | "U32" => count * 4,
            "F16" | "BF16" | "I16" | "U16" => count * 2,
            "F64" => count * 8,
            "I64" | "U64" => count * 8,
            "I8" | "U8" | "BOOL" => count,
            other => return Err(format!("unsupported safetensors dtype '{other}'")),
        };
        if bytes.len() != expected {
            return Err(format!(
                "safetensors tensor '{name}' has {} bytes, expected {expected} for {:?}",
                bytes.len(),
                meta.shape
            ));
        }
        let values = match meta.dtype.as_str() {
            "F32" => fixed_chunks::<4>(&bytes)
                .map(|chunk| f32::from_le_bytes(*chunk))
                .collect(),
            "F16" => fixed_chunks::<2>(&bytes)
                .map(|chunk| half::f16::from_bits(u16::from_le_bytes(*chunk)).to_f32())
                .collect(),
            "BF16" => fixed_chunks::<2>(&bytes)
                .map(|chunk| half::bf16::from_bits(u16::from_le_bytes(*chunk)).to_f32())
                .collect(),
            "F64" => fixed_chunks::<8>(&bytes)
                .map(|chunk| f64::from_le_bytes(*chunk) as f32)
                .collect(),
            "I64" => fixed_chunks::<8>(&bytes)
                .map(|chunk| i64::from_le_bytes(*chunk) as f32)
                .collect(),
            "U64" => fixed_chunks::<8>(&bytes)
                .map(|chunk| u64::from_le_bytes(*chunk) as f32)
                .collect(),
            "I32" => fixed_chunks::<4>(&bytes)
                .map(|chunk| i32::from_le_bytes(*chunk) as f32)
                .collect(),
            "U32" => fixed_chunks::<4>(&bytes)
                .map(|chunk| u32::from_le_bytes(*chunk) as f32)
                .collect(),
            "I16" => fixed_chunks::<2>(&bytes)
                .map(|chunk| i16::from_le_bytes(*chunk) as f32)
                .collect(),
            "U16" => fixed_chunks::<2>(&bytes)
                .map(|chunk| u16::from_le_bytes(*chunk) as f32)
                .collect(),
            "I8" => bytes.iter().map(|value| *value as i8 as f32).collect(),
            "U8" | "BOOL" => bytes.iter().map(|value| *value as f32).collect(),
            _ => unreachable!("dtype was checked above"),
        };
        Ok((meta.shape, values))
    }

    pub fn read_i64_scalar(&mut self, name: &str) -> Result<i64, String> {
        let (meta, bytes) = self.read_bytes(name)?;
        if meta.dtype != "I64" || bytes.len() != 8 {
            return Err(format!(
                "safetensors tensor '{name}' must be a scalar I64 offset"
            ));
        }
        Ok(i64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// Validate that a voice state has the expected per-layer cache layout.
    pub fn validate_pocket_voice(&mut self, layers: usize) -> Result<(), String> {
        for layer in 0..layers {
            let prefix = format!("transformer.layers.{layer}.self_attn");
            for suffix in ["cache", "offset"] {
                let key = format!("{prefix}/{suffix}");
                if !self.contains(&key) {
                    return Err(format!("voice state is missing tensor '{key}'"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_test_file(path: &Path) {
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&2.0f32.to_le_bytes());
        let header = serde_json::json!({
            "a": {
                "dtype": "F32",
                "shape": [2],
                "data_offsets": [0, 8]
            },
            "offset": {
                "dtype": "I64",
                "shape": [1],
                "data_offsets": [8, 16]
            }
        })
        .to_string();
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&(header.len() as u64).to_le_bytes())
            .unwrap();
        file.write_all(header.as_bytes()).unwrap();
        file.write_all(&data).unwrap();
        file.write_all(&127i64.to_le_bytes()).unwrap();
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ttsbard-pocket-safetensors-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn reads_f32_and_i64_tensors() {
        let path = temp_path("read");
        write_test_file(&path);
        let mut file = SafeTensorsFile::open(&path).unwrap();
        assert_eq!(file.read_f32("a").unwrap().1, vec![1.0, 2.0]);
        assert_eq!(file.read_i64_scalar("offset").unwrap(), 127);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn rejects_truncated_header() {
        let path = temp_path("bad");
        std::fs::write(&path, [1, 2, 3]).unwrap();
        assert!(SafeTensorsFile::open(&path).is_err());
        std::fs::remove_file(path).ok();
    }
}
