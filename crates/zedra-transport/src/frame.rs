// L4 Durable Frame Protocol
//
// Defines the framing format for Layer 4 (Durability & Migration).
// L4 frames carry sequence numbers and ACKs to provide exactly-once,
// in-order delivery across transport reconnections.
//
// Frame format (binary, little-endian):
//   seq:     u64  (8 bytes) - sender's sequence number
//   ack_seq: u64  (8 bytes) - piggybacked ACK (highest contiguous seq received)
//   type:    u8   (1 byte)  - frame type discriminant
//   payload: [u8] (variable) - type-specific payload

use anyhow::Result;
use std::io::{Cursor, Read};

/// L4 frame types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    /// RESUME: exchanged on reconnection to sync queue state.
    /// Payload: generation(u32) + last_received_seq(u64)
    Resume = 0x01,
    /// DATA: carries an L5 subchannel frame.
    Data = 0x02,
    /// ACK: standalone acknowledgment (no data payload).
    /// Payload: selective_acks (list of out-of-order seq numbers)
    Ack = 0x03,
    /// RESET: session was lost (e.g. host restarted), start fresh.
    /// Payload: reason string (UTF-8)
    Reset = 0x04,
}

impl FrameType {
    fn from_u8(v: u8) -> Result<Self> {
        match v {
            0x01 => Ok(FrameType::Resume),
            0x02 => Ok(FrameType::Data),
            0x03 => Ok(FrameType::Ack),
            0x04 => Ok(FrameType::Reset),
            _ => anyhow::bail!("unknown L4 frame type: 0x{:02x}", v),
        }
    }
}

/// A decoded L4 frame.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Sender's sequence number for this frame (0 for ACK/RESUME).
    pub seq: u64,
    /// Piggybacked ACK: highest contiguous sequence the sender has received.
    pub ack_seq: u64,
    /// Frame type.
    pub frame_type: FrameType,
    /// Type-specific payload bytes.
    pub payload: Vec<u8>,
}

/// RESUME frame payload.
#[derive(Debug, Clone)]
pub struct ResumePayload {
    /// Transport generation counter.
    pub generation: u32,
    /// Last contiguous sequence number received by this side.
    pub last_received_seq: u64,
}

impl Frame {
    /// Create a DATA frame.
    pub fn data(seq: u64, ack_seq: u64, payload: Vec<u8>) -> Self {
        Self {
            seq,
            ack_seq,
            frame_type: FrameType::Data,
            payload,
        }
    }

    /// Create a standalone ACK frame.
    pub fn ack(ack_seq: u64, selective_acks: &[u64]) -> Self {
        let mut payload = Vec::with_capacity(selective_acks.len() * 8);
        for &s in selective_acks {
            payload.extend_from_slice(&s.to_le_bytes());
        }
        Self {
            seq: 0,
            ack_seq,
            frame_type: FrameType::Ack,
            payload,
        }
    }

    /// Create a RESUME frame.
    pub fn resume(generation: u32, last_received_seq: u64) -> Self {
        let mut payload = Vec::with_capacity(12);
        payload.extend_from_slice(&generation.to_le_bytes());
        payload.extend_from_slice(&last_received_seq.to_le_bytes());
        Self {
            seq: 0,
            ack_seq: 0,
            frame_type: FrameType::Resume,
            payload,
        }
    }

    /// Create a RESET frame.
    pub fn reset(reason: &str) -> Self {
        Self {
            seq: 0,
            ack_seq: 0,
            frame_type: FrameType::Reset,
            payload: reason.as_bytes().to_vec(),
        }
    }

    /// Encode this frame to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let total = 8 + 8 + 1 + self.payload.len();
        let mut buf = Vec::with_capacity(total);
        buf.extend_from_slice(&self.seq.to_le_bytes());
        buf.extend_from_slice(&self.ack_seq.to_le_bytes());
        buf.push(self.frame_type as u8);
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Decode a frame from bytes.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 17 {
            anyhow::bail!(
                "L4 frame too short: {} bytes (minimum 17)",
                data.len()
            );
        }

        let mut cursor = Cursor::new(data);

        let mut seq_buf = [0u8; 8];
        cursor.read_exact(&mut seq_buf)?;
        let seq = u64::from_le_bytes(seq_buf);

        let mut ack_buf = [0u8; 8];
        cursor.read_exact(&mut ack_buf)?;
        let ack_seq = u64::from_le_bytes(ack_buf);

        let mut type_buf = [0u8; 1];
        cursor.read_exact(&mut type_buf)?;
        let frame_type = FrameType::from_u8(type_buf[0])?;

        let mut payload = Vec::new();
        cursor.read_to_end(&mut payload)?;

        Ok(Self {
            seq,
            ack_seq,
            frame_type,
            payload,
        })
    }

    /// Parse a RESUME frame's payload.
    pub fn parse_resume_payload(&self) -> Result<ResumePayload> {
        if self.frame_type != FrameType::Resume {
            anyhow::bail!("not a RESUME frame");
        }
        if self.payload.len() < 12 {
            anyhow::bail!("RESUME payload too short: {} bytes", self.payload.len());
        }
        let generation = u32::from_le_bytes(self.payload[0..4].try_into()?);
        let last_received_seq = u64::from_le_bytes(self.payload[4..12].try_into()?);
        Ok(ResumePayload {
            generation,
            last_received_seq,
        })
    }

    /// Parse selective ACKs from an ACK frame's payload.
    pub fn parse_selective_acks(&self) -> Result<Vec<u64>> {
        if self.frame_type != FrameType::Ack {
            anyhow::bail!("not an ACK frame");
        }
        let mut acks = Vec::new();
        let mut i = 0;
        while i + 8 <= self.payload.len() {
            let s = u64::from_le_bytes(self.payload[i..i + 8].try_into()?);
            acks.push(s);
            i += 8;
        }
        Ok(acks)
    }

    /// Parse a RESET frame's reason string.
    pub fn parse_reset_reason(&self) -> Result<String> {
        if self.frame_type != FrameType::Reset {
            anyhow::bail!("not a RESET frame");
        }
        Ok(String::from_utf8_lossy(&self.payload).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_frame_roundtrip() {
        let frame = Frame::data(42, 10, b"hello world".to_vec());
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.seq, 42);
        assert_eq!(decoded.ack_seq, 10);
        assert_eq!(decoded.frame_type, FrameType::Data);
        assert_eq!(decoded.payload, b"hello world");
    }

    #[test]
    fn ack_frame_roundtrip() {
        let frame = Frame::ack(100, &[103, 105, 107]);
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.seq, 0);
        assert_eq!(decoded.ack_seq, 100);
        assert_eq!(decoded.frame_type, FrameType::Ack);
        let sacks = decoded.parse_selective_acks().unwrap();
        assert_eq!(sacks, vec![103, 105, 107]);
    }

    #[test]
    fn resume_frame_roundtrip() {
        let frame = Frame::resume(5, 1847);
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.frame_type, FrameType::Resume);
        let payload = decoded.parse_resume_payload().unwrap();
        assert_eq!(payload.generation, 5);
        assert_eq!(payload.last_received_seq, 1847);
    }

    #[test]
    fn reset_frame_roundtrip() {
        let frame = Frame::reset("host restarted");
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.frame_type, FrameType::Reset);
        assert_eq!(decoded.parse_reset_reason().unwrap(), "host restarted");
    }

    #[test]
    fn frame_too_short() {
        let short = vec![0u8; 10];
        assert!(Frame::decode(&short).is_err());
    }

    #[test]
    fn unknown_frame_type() {
        let mut frame = Frame::data(1, 0, vec![]).encode();
        frame[16] = 0xFF; // corrupt type byte
        assert!(Frame::decode(&frame).is_err());
    }

    #[test]
    fn empty_data_payload() {
        let frame = Frame::data(1, 0, vec![]);
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.seq, 1);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn ack_no_selective() {
        let frame = Frame::ack(50, &[]);
        let encoded = frame.encode();
        let decoded = Frame::decode(&encoded).unwrap();
        assert_eq!(decoded.ack_seq, 50);
        assert!(decoded.parse_selective_acks().unwrap().is_empty());
    }
}
