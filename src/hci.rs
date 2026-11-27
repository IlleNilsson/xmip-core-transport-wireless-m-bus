//! The host controller interface of IMST's Wireless M-Bus Gateway
//! firmware: what its USB receiver sticks, the iU891A-XL and the iU893A-XL,
//! speak with their host over a serial port (IMST, *Wireless M-Bus Gateway
//! Firmware — Host Controller Interface Protocol*, version 2.4, 2026-06-03,
//! document 4000/40140/0178). The format is IMST's `WiMOD` one, which its
//! other products speak too.
//!
//! A message is a service access point, a message identifier, a payload of
//! at most 500 bytes, and a frame check sequence over them, CRC-16/IBM-SDLC
//! least significant byte first (codec's). On the line each is one SLIP
//! frame, the serial technology's `Framing::Slip`, at 115 200 baud, eight
//! bits, no parity, one stop bit. A request is answered by the identifier
//! after its own, its payload opening with a status; an event comes
//! unasked: a telegram the stick heard is one ([`RECEIVED`]), its time, its
//! decryption and encryption, its mode, its signal strength and the frame
//! plain, which the stick checked the CRCs of and stripped ([`Frame::plain`]).
//! What the host sends goes plain too, the stick putting on the length and
//! the CRCs.

use std::time::Duration;

use codec::crc::CRC_16_IBM_SDLC;
use transport::TransportError;
use transport::error::{Result, protocol_error};

use crate::frame::Frame;

/// The rate the stick's serial port runs at.
pub const BAUD: u32 = 115_200;
/// How long a request's response is waited for: within "very short time
/// ( less than a second )", as the specification has it.
pub const ANSWERED_WITHIN: Duration = Duration::from_secs(1);
/// The most payload a message carries.
pub const MAX_PAYLOAD: usize = 500;

/// The device management services' access point.
pub const DEVICE_MANAGEMENT: u8 = 0x01;
/// A ping, answered by its response.
pub const PING: u8 = 0x01;
/// The WM-Bus gateway services' access point.
pub const GATEWAY: u8 = 0x09;
/// The active configuration asked for.
pub const GET_ACTIVE_CONFIGURATION: u8 = 0x01;
/// The active configuration set, kept until the stick restarts.
pub const SET_ACTIVE_CONFIGURATION: u8 = 0x03;
/// A telegram the stick heard: an event.
pub const RECEIVED: u8 = 0x20;
/// A telegram sent as the host wrote it, unencrypted.
pub const SEND: u8 = 0x31;
/// A telegram sent: an event, where the configuration asks for one.
pub const TRANSMITTED: u8 = 0x34;

/// The configuration's option bit that hands the host only the telegrams
/// of the meters on the stick's own list.
pub const ADDRESS_FILTER: u16 = 0x0001;
/// The configuration's option bit that hands the host every telegram heard.
pub const NOTIFY_RECEIVED: u16 = 0x0002;
/// The configuration's option bit that tells the host each telegram sent.
pub const NOTIFY_SENT: u16 = 0x0004;
/// The configuration's link mode that turns the receiver off.
pub const OFF: u8 = 0;

/// The status of a request carried out.
pub const OK: u8 = 0x00;
/// The status of a request the firmware does not carry out at all.
pub const UNSUPPORTED: u8 = 0x02;
/// The status of a stick busy, to be asked again.
const BUSY: u8 = 0x06;

/// The mode of EN 13757-4 a meter speaks, as a Location names it: what
/// the stick listens in and what it sends to the meter in, format A.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Stationary: 32.768 kchip/s both ways.
    S,
    /// Frequent transmit: the meter at 100 kchip/s, the other device
    /// answering at S's 32.768 kchip/s.
    T,
    /// Compact: the meter at 100 kbit/s, the other device answering at
    /// 50 kbit/s.
    C,
}

impl Mode {
    /// The mode a Location's `mode` names: `s`, `t` or `c`.
    ///
    /// # Errors
    /// Any other name.
    pub fn named(name: &str) -> Result<Self> {
        match name {
            "s" => Ok(Self::S),
            "t" => Ok(Self::T),
            "c" => Ok(Self::C),
            other => Err(protocol_error(format!("{other:?} is no mode: s, t or c"))),
        }
    }

    /// The link mode the stick listens in: S, or C and T at once.
    #[must_use]
    pub const fn listening(self) -> u8 {
        match self {
            Self::S => 1,
            Self::T | Self::C => 3,
        }
    }

    /// The packet mode a telegram to the meter goes in, format A: S for S
    /// and T, C at 50 kbit/s for C.
    #[must_use]
    pub const fn sending(self) -> u8 {
        match self {
            Self::S | Self::T => 0x01,
            Self::C => 0x04,
        }
    }

    /// The packet information an event gives a telegram heard from a
    /// meter in this mode, C's at 100 kbit/s in format A.
    #[must_use]
    pub const fn heard(self) -> u8 {
        match self {
            Self::S => 1,
            Self::T => 2,
            Self::C => 5,
        }
    }
}

/// One message, either way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The service access point: [`DEVICE_MANAGEMENT`], [`GATEWAY`].
    pub sap: u8,
    /// What the message is, within its access point.
    pub id: u8,
    pub payload: Vec<u8>,
}

impl Message {
    #[must_use]
    pub const fn new(sap: u8, id: u8, payload: Vec<u8>) -> Self {
        Self { sap, id, payload }
    }

    /// The message's bytes, its frame check sequence after them, least
    /// significant byte first.
    ///
    /// # Errors
    /// A payload over [`MAX_PAYLOAD`].
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(protocol_error(format!(
                "{} bytes of payload is over a message's {MAX_PAYLOAD}",
                self.payload.len()
            )));
        }
        let mut bytes = vec![self.sap, self.id];
        bytes.extend_from_slice(&self.payload);
        let check = CRC_16_IBM_SDLC.checksum(&bytes);
        bytes.extend_from_slice(&check.to_le_bytes());
        Ok(bytes)
    }

    /// The message `bytes` carry, its frame check sequence checked.
    ///
    /// # Errors
    /// Too short for a message, or a check that does not match.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let split = bytes.len().checked_sub(2);
        let Some((message, check)) = split.and_then(|at| bytes.split_at_checked(at)) else {
            return Err(protocol_error("a message shorter than its check"));
        };
        let [sap, id, payload @ ..] = message else {
            return Err(protocol_error(
                "a message with no access point or identifier",
            ));
        };
        if CRC_16_IBM_SDLC.checksum(message).to_le_bytes() != check {
            return Err(protocol_error("a message whose check does not match"));
        }
        Ok(Self::new(*sap, *id, payload.to_vec()))
    }

    /// Whether this is the response to `request`: its access point, and
    /// the identifier after its.
    #[must_use]
    pub fn answers(&self, request: &Self) -> bool {
        self.sap == request.sap && self.id == request.id.wrapping_add(1)
    }

    /// The response to `request` with `status`, then `rest`.
    #[must_use]
    pub fn response(request: &Self, status: u8, rest: &[u8]) -> Self {
        let mut payload = vec![status];
        payload.extend_from_slice(rest);
        Self::new(request.sap, request.id.wrapping_add(1), payload)
    }

    /// What follows a response's status, once it says the request was
    /// carried out.
    ///
    /// # Errors
    /// A status that says it was not, in its words, worth asking again
    /// where the stick was busy; or no status.
    pub fn granted(&self) -> Result<&[u8]> {
        match self.payload.split_first() {
            Some((&OK, rest)) => Ok(rest),
            Some((&BUSY, _)) => Err(TransportError::retryable("the receiver is busy")),
            Some((status, _)) => Err(protocol_error(format!(
                "the receiver refused: {}",
                refusal(*status)
            ))),
            None => Err(protocol_error("a response with no status")),
        }
    }

    /// The request that sends `frame` in packet `mode` at `power` dBm: the
    /// frame plain, less the length the stick puts on.
    ///
    /// # Errors
    /// A frame over a telegram's data.
    pub fn send(frame: &Frame, mode: u8, power: i8) -> Result<Self> {
        let mut payload = vec![mode, power.to_le_bytes()[0]];
        payload.extend_from_slice(&frame.plain()?[1..]);
        Ok(Self::new(GATEWAY, SEND, payload))
    }

    /// The frame a send request carries, as the stick reads it.
    ///
    /// # Errors
    /// No send request, or one too short for a frame.
    pub fn sent(&self) -> Result<Frame> {
        match (self.sap, self.id, self.payload.get(2..)) {
            (GATEWAY, SEND, Some(content)) => {
                let length = u8::try_from(content.len())
                    .map_err(|_| protocol_error("a telegram over its length"))?;
                Frame::from_plain(&[&[length][..], content].concat())
            }
            _ => Err(protocol_error("no telegram to send")),
        }
    }

    /// The event of a stick that heard `frame` in packet `mode` at `rssi`
    /// dBm: no time, nothing decrypted, nothing encrypted.
    ///
    /// # Errors
    /// A frame over a telegram's data.
    pub fn received(frame: &Frame, mode: u8, rssi: i8) -> Result<Self> {
        let mut payload = vec![0, 0, 0, 0, 0, 0, mode, rssi.to_le_bytes()[0]];
        payload.extend(frame.plain()?);
        Ok(Self::new(GATEWAY, RECEIVED, payload))
    }

    /// The telegram this event says the stick heard, where it is one: the
    /// frame past the event's time, decryption, encryption, mode and signal
    /// strength.
    ///
    /// # Errors
    /// An event too short for a frame, or a frame that is no frame.
    pub fn heard(&self) -> Result<Option<Frame>> {
        if (self.sap, self.id) != (GATEWAY, RECEIVED) {
            return Ok(None);
        }
        let plain = self
            .payload
            .get(8..)
            .ok_or_else(|| protocol_error("a telegram's event too short for a telegram"))?;
        Frame::from_plain(plain).map(Some)
    }
}

/// A status's words, as the WM-Bus gateway services' table gives them.
const fn refusal(status: u8) -> &'static str {
    match status {
        0x01 => "error",
        0x02 => "command not supported",
        0x03 => "wrong parameter",
        0x04 => "wrong application mode",
        0x05 => "no more data",
        0x07 => "wrong message length",
        0x08 => "non-volatile memory write error",
        0x09 => "non-volatile memory read error",
        0x0a => "command rejected in the current application state",
        0x0b => "access denied in the current operation mode",
        0x0c => "data truncated",
        0x0d => "unsupported encryption mode",
        _ => "a status the specification does not list",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::SND_UD;

    /// The Ping request and response as the specification writes them,
    /// SLIP's ends taken off: two bytes, then the check, least significant
    /// byte first.
    #[test]
    fn the_specifications_ping_encodes_and_decodes_byte_for_byte() {
        let ping = Message::new(DEVICE_MANAGEMENT, PING, Vec::new());
        assert_eq!(ping.encode().expect("encode"), [0x01, 0x01, 0x16, 0x07]);
        let answer = Message::decode(&[0x01, 0x02, 0x00, 0xa0, 0xaf]).expect("decode");
        assert!(answer.answers(&ping));
        assert_eq!(answer.granted().expect("ok"), [] as [u8; 0]);
        assert_eq!(Message::response(&ping, OK, &[]), answer);
        assert!(
            Message::decode(&[0x01, 0x02, 0x00, 0xa0, 0xae]).is_err(),
            "the check"
        );
        assert!(Message::decode(&[0x01, 0x16]).is_err(), "no identifier");
        assert!(Message::decode(&[0x01]).is_err(), "shorter than its check");
        let long = Message::new(GATEWAY, SEND, vec![0; MAX_PAYLOAD + 1]);
        assert!(long.encode().is_err(), "over the payload");
    }

    #[test]
    fn a_refusal_is_said_in_the_specifications_words() {
        let request = Message::new(GATEWAY, SEND, Vec::new());
        let refused = Message::response(&request, 0x03, &[]);
        let error = refused.granted().expect_err("refused");
        assert!(error.message.contains("wrong parameter"), "{error}");
        assert!(!error.retryable);
        let busy = Message::response(&request, BUSY, &[])
            .granted()
            .expect_err("busy");
        assert!(busy.retryable, "a busy stick is asked again");
        let unlisted = Message::response(&request, 0x42, &[])
            .granted()
            .expect_err("unlisted");
        assert!(unlisted.message.contains("does not list"), "{unlisted}");
        let empty = Message::new(GATEWAY, 0x32, Vec::new());
        assert!(empty.granted().is_err(), "no status");
    }

    /// A send request carries the frame plain less its length, and an
    /// event the frame plain whole past eight bytes of what it knows of it.
    #[test]
    fn a_telegram_is_sent_and_heard_plain() {
        let frame = Frame {
            control: SND_UD,
            address: [0xb0, 0x61, 0x78, 0x56, 0x34, 0x12, 1, 7],
            ci: 0x51,
            data: vec![1, 2, 3],
        };
        let send = Message::send(&frame, Mode::C.sending(), -1).expect("send");
        assert_eq!(&send.payload[..3], [0x04, 0xff, SND_UD]);
        assert_eq!(send.payload.len(), 2 + 13, "no length, no CRC");
        assert_eq!(send.sent().expect("read back"), frame);
        let heard = Message::received(&frame, Mode::T.heard(), -60).expect("event");
        assert_eq!(
            heard.payload[6..9],
            [2, 0xc4, 13],
            "mode, signal, the length"
        );
        assert_eq!(heard.heard().expect("heard"), Some(frame));
        assert_eq!(send.heard().expect("no event"), None);
        let short = Message::new(GATEWAY, RECEIVED, vec![0; 7]);
        assert!(short.heard().is_err(), "too short for a frame");
        let acked = Message::new(GATEWAY, RECEIVED, vec![0; 8]);
        assert!(acked.heard().is_err(), "no frame");
        assert!(heard.sent().is_err(), "an event is no send");
    }

    #[test]
    fn a_locations_mode_is_what_the_stick_listens_and_sends_in() {
        assert_eq!(Mode::named("t").expect("t"), Mode::T);
        assert_eq!(Mode::named("c").expect("c").sending(), 0x04);
        assert_eq!(Mode::named("s").expect("s").listening(), 1);
        assert_eq!(Mode::T.listening(), Mode::C.listening(), "C and T at once");
        assert!(Mode::named("T1").is_err());
    }
}
