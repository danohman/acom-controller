//! Wire framing: `0x55 | address | length | data... | checksum`.
//!
//! `length` counts every byte of the frame (start, address, length, data and
//! checksum). The checksum is chosen so the byte-sum of the whole frame is
//! zero modulo 256.

pub const START: u8 = 0x55;
pub const MIN_LEN: usize = 4;
pub const MAX_LEN: usize = 255;

/// A validated frame: address plus payload (bytes between length and checksum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub address: u8,
    pub data: Vec<u8>,
}

/// Compute the checksum byte for the given leading bytes.
pub fn checksum(bytes: &[u8]) -> u8 {
    let sum = bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    0u8.wrapping_sub(sum)
}

/// Serialise a frame for transmission.
pub fn encode(address: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len() + MIN_LEN;
    debug_assert!(len <= MAX_LEN);
    let mut out = Vec::with_capacity(len);
    out.push(START);
    out.push(address);
    out.push(len as u8);
    out.extend_from_slice(data);
    out.push(checksum(&out));
    out
}

/// Incremental stream decoder. Feed it arbitrary chunks of bytes; it yields
/// complete, checksum-valid frames and resynchronises after corruption.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// Count of frames discarded because of a bad checksum or length.
    pub errors: u64,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop any partial frame. Call after an idle gap on the line (longer than
    /// the 20 ms inter-message pause) so a false start tag cannot stall decoding.
    pub fn reset(&mut self) {
        self.buf.clear();
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            // Hunt for the start tag.
            match self.buf.iter().position(|&b| b == START) {
                None => {
                    self.buf.clear();
                    break;
                }
                Some(0) => {}
                Some(n) => {
                    self.buf.drain(..n);
                }
            }
            if self.buf.len() < 3 {
                break;
            }
            let len = self.buf[2] as usize;
            if len < MIN_LEN {
                // Impossible length: this 0x55 was noise or data. Skip it.
                self.errors += 1;
                self.buf.remove(0);
                continue;
            }
            if self.buf.len() < len {
                break; // wait for more bytes
            }
            let sum = self.buf[..len].iter().fold(0u8, |a, b| a.wrapping_add(*b));
            if sum == 0 {
                out.push(Frame {
                    address: self.buf[1],
                    data: self.buf[3..len - 1].to_vec(),
                });
                self.buf.drain(..len);
            } else {
                self.errors += 1;
                self.buf.remove(0);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_example_bad_message_reply() {
        // Spec 4.1: 55 01 04 A6
        assert_eq!(encode(0x01, &[]), vec![0x55, 0x01, 0x04, 0xA6]);
    }

    #[test]
    fn spec_example_disable_telemetry() {
        // Spec 6.7: 55 91 04 16
        assert_eq!(encode(0x91, &[]), vec![0x55, 0x91, 0x04, 0x16]);
        // 6.8: 55 92 04 15, 6.9: 55 93 04 14, 6.6: 55 90 04 17
        assert_eq!(encode(0x92, &[]), vec![0x55, 0x92, 0x04, 0x15]);
        assert_eq!(encode(0x93, &[]), vec![0x55, 0x93, 0x04, 0x14]);
        assert_eq!(encode(0x90, &[]), vec![0x55, 0x90, 0x04, 0x17]);
    }

    #[test]
    fn roundtrip_and_resync() {
        let a = encode(0x2F, &[1, 2, 3, 4]);
        let b = encode(0x03, &[0, 9]);
        let mut d = Decoder::new();
        let mut frames = Vec::new();
        // Garbage containing a false start tag, then an idle gap (reset).
        d.push(&[0x00, 0x55, 0x55, 0x01]);
        d.reset();
        let mut stream = vec![0x00, 0x13]; // plain noise
        stream.extend_from_slice(&a);
        stream.extend_from_slice(&b);
        // dribble the bytes in one at a time
        for byte in stream {
            frames.extend(d.push(&[byte]));
        }
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].address, 0x2F);
        assert_eq!(frames[0].data, vec![1, 2, 3, 4]);
        assert_eq!(frames[1].address, 0x03);
    }

    #[test]
    fn rejects_corruption() {
        let mut a = encode(0x2F, &[1, 2, 3, 4]);
        a[4] ^= 0xFF;
        let mut d = Decoder::new();
        assert!(d.push(&a).is_empty());
        assert!(d.errors > 0);
    }
}
