//! The link layer of wireless M-Bus, EN 13757-4 frame format A: a length,
//! a control field, the meter's manufacturer and address, the control
//! information, and a CRC over every block.
//!
//! The first block is ten bytes — the length, the control field, two
//! bytes of manufacturer and six of address — and its CRC; every block
//! after is sixteen bytes and its CRC, the last as long as what is left.
//! The length counts everything from the control field to the end of the
//! user data and none of the CRCs, so a frame carries at most 245 bytes
//! past its control information. The CRC is CRC-16/EN-13757, codec's.

use codec::crc::CRC_16_EN_13757;
use codec::hex;
use transport::error::{Result, protocol_error};

/// The meter sends, expecting no reply.
pub const SND_NR: u8 = 0x44;
/// The meter acknowledges.
pub const ACK: u8 = 0x00;
/// The other device initialises the meter.
pub const SND_NKE: u8 = 0x40;
/// The other device sends user data to the meter.
pub const SND_UD: u8 = 0x53;
/// The other device asks for class 2 data.
pub const REQ_UD2: u8 = 0x5b;
/// The meter answers with its data.
pub const RSP_UD: u8 = 0x08;

/// The most user data a frame carries past its control information: the
/// length byte less control, manufacturer, address and control information.
pub const MAX_DATA: usize = 245;
/// The address every meter listens at.
pub const BROADCAST: [u8; 8] = [0xff; 8];
/// A block past the first.
const BLOCK: usize = 16;

/// One frame on the air.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub control: u8,
    /// Manufacturer and address, as [`m_bus::Identity::link_address`]
    /// lays them out.
    pub address: [u8; 8],
    /// The control information: what the user data is.
    pub ci: u8,
    pub data: Vec<u8>,
}

impl Frame {
    /// The bytes on the air, a CRC after every block.
    ///
    /// # Errors
    /// More than [`MAX_DATA`] bytes of data.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.data.len() > MAX_DATA {
            return Err(protocol_error(format!(
                "{} bytes of user data is over a wireless frame's {MAX_DATA}",
                self.data.len()
            )));
        }
        let length = u8::try_from(10 + self.data.len()).unwrap_or(u8::MAX);
        let mut first = vec![length, self.control];
        first.extend_from_slice(&self.address);
        let mut out = first.clone();
        out.extend_from_slice(&CRC_16_EN_13757.checksum(&first).to_be_bytes());
        let mut rest = vec![self.ci];
        rest.extend_from_slice(&self.data);
        for block in rest.chunks(BLOCK) {
            out.extend_from_slice(block);
            out.extend_from_slice(&CRC_16_EN_13757.checksum(block).to_be_bytes());
        }
        Ok(out)
    }

    /// The frame `bytes` carry, every CRC checked.
    ///
    /// # Errors
    /// Too short for its first block, a length that does not match, or a
    /// CRC that does not.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (first, rest) = bytes
            .split_at_checked(12)
            .ok_or_else(|| protocol_error("a frame shorter than its first block"))?;
        checked(first)?;
        let length = usize::from(first[0]);
        if length < 10 {
            return Err(protocol_error("a length shorter than the first block"));
        }
        let mut body = Vec::with_capacity(length - 9);
        let mut rest = rest;
        while body.len() < length - 9 {
            let want = (length - 9 - body.len()).min(BLOCK);
            let (block, after) = rest
                .split_at_checked(want + 2)
                .ok_or_else(|| protocol_error("a frame cut short inside a block"))?;
            body.extend_from_slice(checked(block)?);
            rest = after;
        }
        if !rest.is_empty() {
            return Err(protocol_error("bytes after the last block"));
        }
        let mut address = [0u8; 8];
        address.copy_from_slice(&first[2..10]);
        Ok(Self {
            control: first[1],
            address,
            ci: body[0],
            data: body[1..].to_vec(),
        })
    }
}

/// `block` without its two CRC bytes, once they check.
fn checked(block: &[u8]) -> Result<&[u8]> {
    let (data, sum) = block
        .split_at_checked(block.len().saturating_sub(2))
        .ok_or_else(|| protocol_error("a block with no CRC"))?;
    if CRC_16_EN_13757.checksum(data).to_be_bytes() != sum {
        return Err(protocol_error("a block whose CRC does not check"));
    }
    Ok(data)
}

/// `<manufacturer>-<ident>` for a link address, as a Location names the
/// meter: the three letters unpacked from five bits each, the eight
/// digits unpacked from BCD.
#[must_use]
pub fn label(address: &[u8; 8]) -> String {
    let code = u16::from_le_bytes([address[0], address[1]]);
    let letters: String = [10, 5, 0]
        .into_iter()
        .map(|shift| char::from(u8::try_from((code >> shift) & 0x1f).unwrap_or(0) + 64))
        .collect();
    let bcd: Vec<u8> = address[2..6].iter().rev().copied().collect();
    format!("{letters}-{}", hex::encode(&bcd))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: [u8; 8] = [0xb0, 0x61, 0x78, 0x56, 0x34, 0x12, 1, 7];

    #[test]
    fn a_frame_encodes_with_a_crc_a_block_and_decodes_back() {
        let frame = Frame {
            control: SND_NR,
            address: ADDRESS,
            ci: 0x7a,
            data: (0..40).collect(),
        };
        let bytes = frame.encode().expect("encode");
        assert_eq!(bytes[0], 50, "the length counts no CRC");
        assert_eq!(
            bytes.len(),
            12 + 18 + 18 + 11,
            "three blocks past the first"
        );
        assert_eq!(Frame::decode(&bytes).expect("decode"), frame);
        let empty = Frame {
            control: ACK,
            address: ADDRESS,
            ci: 0x7a,
            data: vec![],
        };
        let bytes = empty.encode().expect("encode");
        assert_eq!(bytes.len(), 12 + 3);
        assert_eq!(Frame::decode(&bytes).expect("decode"), empty);
        let full = Frame {
            control: SND_UD,
            address: BROADCAST,
            ci: 0x51,
            data: vec![9; MAX_DATA],
        };
        assert_eq!(
            Frame::decode(&full.encode().expect("encode")).expect("decode"),
            full
        );
        assert!(
            Frame {
                data: vec![9; MAX_DATA + 1],
                ..full
            }
            .encode()
            .is_err()
        );
    }

    #[test]
    fn a_frame_whose_blocks_do_not_check_is_refused() {
        let frame = Frame {
            control: RSP_UD,
            address: ADDRESS,
            ci: 0x7a,
            data: vec![1, 2, 3],
        };
        let good = frame.encode().expect("encode");
        let mut first_block = good.clone();
        first_block[3] ^= 0x01;
        assert!(Frame::decode(&first_block).is_err(), "the first CRC");
        let mut second_block = good.clone();
        second_block[14] ^= 0x01;
        assert!(Frame::decode(&second_block).is_err(), "the second CRC");
        assert!(Frame::decode(&good[..12]).is_err(), "cut short");
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(
            Frame::decode(&trailing).is_err(),
            "bytes after the last block"
        );
        assert!(Frame::decode(&[0; 5]).is_err(), "shorter than a block");
        let mut short_length = good;
        short_length[0] = 9;
        assert!(
            Frame::decode(&short_length).is_err(),
            "the CRC catches the length"
        );
    }

    #[test]
    fn a_link_address_reads_back_as_the_meter_label() {
        assert_eq!(label(&ADDRESS), "XMP-12345678");
        assert_eq!(label(&[0x41, 0x04, 0x01, 0, 0, 0, 0, 0]), "ABA-00000001");
    }
}
