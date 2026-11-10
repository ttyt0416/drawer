//! Streaming native payload integrity; does not serialize session Undo stacks.
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
const END: &[u8; 8] = b"DWEND1\r\n";
fn bad(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
pub(crate) struct Reader<R> {
    inner: R,
    hash: Sha256,
    pub count: u64,
    limit: u64,
}
impl<R: Read> Reader<R> {
    pub fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
            count: 0,
            limit,
        }
    }
    pub fn finish(&mut self, version: u32) -> io::Result<Option<(u64, u32)>> {
        if version < 124 {
            return Ok(None);
        }
        if self.count != self.limit {
            return Err(bad("Unexpected data before native integrity footer"));
        }
        let mut footer = [0; 64];
        self.inner.read_exact(&mut footer).map_err(|e| {
            bad(&format!(
                "Native integrity footer at byte {}: {e}",
                self.count
            ))
        })?;
        if &footer[..8] != END {
            return Err(bad("Native integrity footer marker mismatch"));
        }
        let length = u64::from_le_bytes(footer[8..16].try_into().unwrap());
        let mut hash = self.hash.clone();
        hash.update(&footer[16..32]);
        if footer[28..32] != [0; 4] || length != self.count || footer[32..] != hash.finalize()[..] {
            return Err(bad("Native payload length/SHA256 mismatch"));
        }
        Ok(Some((
            u64::from_le_bytes(footer[16..24].try_into().unwrap()),
            u32::from_le_bytes(footer[24..28].try_into().unwrap()),
        )))
    }
}
impl<R: Read> Read for Reader<R> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let available = (self.limit - self.count).min(b.len() as u64) as usize;
        let n = self.inner.read(&mut b[..available])?;
        self.hash.update(&b[..n]);
        self.count = self
            .count
            .checked_add(n as u64)
            .ok_or_else(|| bad("Native byte count overflow"))?;
        Ok(n)
    }
}
pub(crate) struct Writer<W> {
    inner: W,
    hash: Sha256,
    count: u64,
}
impl<W: Write> Writer<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
            count: 0,
        }
    }
    pub fn finish(mut self, revision: u64, frame: u32) -> io::Result<()> {
        let mut footer = END.to_vec();
        footer.extend(self.count.to_le_bytes());
        footer.extend(revision.to_le_bytes());
        footer.extend(frame.to_le_bytes());
        footer.extend(0u32.to_le_bytes());
        self.hash.update(&footer[16..32]);
        footer.extend(self.hash.finalize());
        self.inner.write_all(&footer)
    }
}
impl<W: Write> Write for Writer<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(b)?;
        self.hash.update(&b[..n]);
        self.count = self
            .count
            .checked_add(n as u64)
            .ok_or_else(|| bad("Native byte count overflow"))?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
