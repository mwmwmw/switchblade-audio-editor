//! Tiny little-endian binary encoder/decoder for the sidecar cache.

use anyhow::{anyhow, Result};

#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn f32(&mut self, value: f32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn f64(&mut self, value: f64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn f32_slice(&mut self, values: &[f32]) {
        self.u64(values.len() as u64);
        for value in values {
            self.f32(*value);
        }
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    pub fn is_at_end(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len());
        let end = end.ok_or_else(|| anyhow!("cache truncated at byte {}", self.position))?;
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    pub fn bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        self.take(count)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }

    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into()?))
    }

    pub fn f32_vec(&mut self) -> Result<Vec<f32>> {
        let count = self.len_prefix()?;
        (0..count).map(|_| self.f32()).collect()
    }

    /// Length prefixes are validated against the remaining bytes so a corrupt file cannot force a huge allocation.
    pub fn len_prefix(&mut self) -> Result<usize> {
        let count = self.u64()? as usize;
        if count > self.bytes.len() - self.position {
            return Err(anyhow!("cache length prefix {count} exceeds file size"));
        }
        Ok(count)
    }
}
