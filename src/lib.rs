#![forbid(unsafe_code)]

//! Streams that cross the air as wireless M-Bus telegrams, the records
//! and the meter of wired M-Bus under the frame of EN 13757-4.
//!
//! Wireless M-Bus is the meter on the wall talking to the concentrator in
//! the street: 868 MHz in Europe, a meter transmitting on its own in mode
//! T or S, or asked and answering in mode C. What is here is the link
//! layer — a length, a control field, the meter's manufacturer and address,
//! the control information, a CRC on every block — over the same records
//! and the same meter as [`m_bus`](m_bus): a Stream as variable-length
//! records over as many telegrams as it takes, written by `SND_UD` and
//! acknowledged, read back by `REQ_UD2` and `RSP_UD`, or taken as the
//! meter sends it unasked in `SND_NR`. There is no ceiling.
//!
//! The air is a [`Line`], the boundary wired M-Bus has for its wire: a
//! deployment's is a radio receiver or a gateway on a serial port; the one
//! here is a loopback radio with a meter listening, so a concentrator and
//! a meter round-trip in process with no antenna, which is what
//! [`WirelessMBusTransport::loopback`] stands up (ADR-0051), as
//! wireless-hart is to hart.
//!
//! **What a meter sends unasked is at-most-once** ([`AT_MOST_ONCE`]):
//! `SND_NR` has no reply. **What a receive reads by `REQ_UD2` consumes
//! nothing** — the meter keeps holding it — so its verdict has nothing to
//! tell the meter, whichever it is, and a cycle that did not complete loses
//! nothing.
//!
//! The origin URI names the air and the meter:
//! `wmbus://<air>/<manufacturer>-<ident>`.

pub mod frame;
pub mod loopback;

use std::sync::Arc;
use std::time::Duration;

use m_bus::record::{self, CI_DATA_SEND, CI_VARIABLE_SHORT};
use transport::error::{Result, protocol_error};
use transport::{Acknowledgement, Arrived, Configured, Directions, Taken, Transport};
use xcore::settings::{Applies, Fixed, Kind, Presence, Read, Setting, Settings};

/// Why a Stream a meter sends unasked cannot be acknowledged after the
/// receive cycle.
pub const AT_MOST_ONCE: &str = "a wireless M-Bus meter sends SND_NR unasked and waits for no \
                                reply: the telegram is off the air as it is heard";

pub use frame::{Frame, MAX_DATA};
pub use m_bus::{Identity, Meter};
use transport::line::Line;

use crate::frame::{ACK, REQ_UD2, RSP_UD, SND_NKE, SND_NR, SND_UD};

/// The room a meter's telegram leaves for records: the frame's data less
/// the short header.
pub const ANSWER_ROOM: usize = MAX_DATA - 4;

/// How long a device waits for a meter's answer unless told otherwise.
pub const TIMEOUT: Duration = Duration::from_secs(1);

/// A byte-sized part of a meter's identity.
const BYTE: Kind = Kind::Integer {
    minimum: 0,
    maximum: 255,
};

/// The other device's side of the air: a concentrator, a gateway, a
/// hand-held reader.
#[derive(Clone)]
pub struct WirelessMBusTransport {
    air: Arc<dyn Line>,
    address: [u8; 8],
    timeout: Duration,
}

impl WirelessMBusTransport {
    /// A device on `air`, talking to the meter at link `address`.
    #[must_use]
    pub fn new(air: Arc<dyn Line>, address: [u8; 8]) -> Self {
        Self {
            air,
            address,
            timeout: TIMEOUT,
        }
    }

    /// Give up on a meter that does not answer within `timeout`.
    #[must_use]
    pub const fn timing_out_after(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `wmbus://<air>/<manufacturer>-<ident>`.
    #[must_use]
    pub fn origin(&self, address: &[u8; 8]) -> String {
        format!("wmbus://{}/{}", self.air.name(), frame::label(address))
    }

    /// Transmit `control` with `ci` and `data` to the meter, and take its
    /// answer.
    ///
    /// # Errors
    /// No answer in time, or an answer that is no frame.
    pub fn exchange(&self, control: u8, ci: u8, data: Vec<u8>) -> Result<Frame> {
        let frame = Frame {
            control,
            address: self.address,
            ci,
            data,
        };
        self.air.transmit(&frame.encode()?)?;
        let answer = self
            .air
            .receive(self.timeout)?
            .ok_or_else(|| transport::TransportError::retryable("the meter did not answer"))?;
        Frame::decode(&answer)
    }

    /// `SND_NKE`: initialise the meter.
    ///
    /// # Errors
    /// No acknowledgement.
    pub fn initialise(&self) -> Result<()> {
        acknowledged(&self.exchange(SND_NKE, CI_DATA_SEND, Vec::new())?)
    }

    /// Write `bytes` to the meter, a telegram of records at a time, each
    /// acknowledged.
    ///
    /// # Errors
    /// A telegram not acknowledged.
    pub fn write_stream(&self, bytes: &[u8]) -> Result<()> {
        for data in record::pack(bytes, MAX_DATA)? {
            acknowledged(&self.exchange(SND_UD, CI_DATA_SEND, data)?)?;
        }
        Ok(())
    }

    /// Read the Stream the meter holds, whole, a telegram of records at a
    /// time. The meter keeps holding it.
    ///
    /// # Errors
    /// An answer that is not the meter's data, or records that are no
    /// Stream.
    pub fn read_stream(&self) -> Result<Taken> {
        let mut bytes = Vec::new();
        loop {
            let answer = self.exchange(REQ_UD2, CI_DATA_SEND, Vec::new())?;
            let (chunk, more) = records_of(&answer, RSP_UD)?;
            bytes.extend_from_slice(&chunk);
            if !more {
                return Ok(Taken::new(self.origin(&answer.address), bytes));
            }
        }
    }

    /// Take what a meter sends unasked, whole: the `SND_NR` telegrams of
    /// one Stream, as many as it takes, or `None` when the air is quiet.
    ///
    /// # Errors
    /// A frame that is not a meter's transmission, or records that are no
    /// Stream.
    pub fn listen(&self) -> Result<Option<Taken>> {
        let mut bytes = Vec::new();
        let mut from = None;
        loop {
            let Some(heard) = self.air.receive(self.timeout)? else {
                return Ok(from.map(|address| {
                    Taken::new(
                        format!("{}?unsolicited=true", self.origin(&address)),
                        std::mem::take(&mut bytes),
                    )
                }));
            };
            let frame = Frame::decode(&heard)?;
            let (chunk, more) = records_of(&frame, SND_NR)?;
            bytes.extend_from_slice(&chunk);
            from = Some(frame.address);
            if !more {
                return Ok(Some(Taken::new(
                    format!("{}?unsolicited=true", self.origin(&frame.address)),
                    bytes,
                )));
            }
        }
    }
}

/// The records a meter's frame carries past its short header, if it is
/// `control`.
fn records_of(frame: &Frame, control: u8) -> Result<(Vec<u8>, bool)> {
    if frame.control != control || frame.ci != CI_VARIABLE_SHORT {
        return Err(protocol_error(format!(
            "control {:#04x} with information {:#04x} is not the meter's data",
            frame.control, frame.ci
        )));
    }
    record::unpack(frame.data.get(4..).unwrap_or(&[]))
}

fn acknowledged(answer: &Frame) -> Result<()> {
    if answer.control == ACK {
        Ok(())
    } else {
        Err(protocol_error(format!(
            "control {:#04x} where an acknowledgement was due",
            answer.control
        )))
    }
}

impl Transport for WirelessMBusTransport {
    fn name(&self) -> &'static str {
        "wireless-m-bus"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    fn arrivals(&self) -> transport::Arrivals {
        transport::Arrivals::Ordered("a poll reads again what is not yet told")
    }

    /// What the meter sends unasked if it has; the Stream it holds
    /// otherwise. Each arrives whole. Sent unasked, `SND_NR` has no reply:
    /// acceptance is at-most-once ([`AT_MOST_ONCE`]). Read by `REQ_UD2`, the
    /// verdict has nothing to tell the meter, whichever it is: the read
    /// consumes nothing, so a cycle that did not complete loses nothing —
    /// the next read finds the Stream again.
    fn receive(&self) -> Result<Vec<Arrived>> {
        let (taken, acknowledgement) = match self.listen()? {
            Some(heard) => (heard, Acknowledgement::at_most_once(AT_MOST_ONCE)),
            None => (self.read_stream()?, Acknowledgement::unconsumed()),
        };
        Ok(vec![Arrived::whole(
            taken.origin_uri,
            taken.bytes,
            acknowledgement,
        )])
    }

    /// Write to the meter this device is configured for.
    fn send(&self, _target: &str, bytes: &[u8]) -> Result<()> {
        self.write_stream(bytes)
    }
}

impl Configured for WirelessMBusTransport {
    /// The address names the air. `loopback`, a meter on an in-process
    /// radio, is the one the estate has; a receiver joins when it exposes
    /// one. The meter is named by the four parts of its link address.
    const SETTINGS: &'static Settings = &Settings {
        technology: env!("CARGO_PKG_NAME"),
        settings: &[
            Setting {
                name: "manufacturer",
                kind: Kind::Text,
                presence: Presence::Required,
                meaning: "The meter's manufacturer, three capital letters.",
                applies: Applies::Both,
            },
            Setting {
                name: "ident",
                kind: Kind::Integer {
                    minimum: 0,
                    maximum: 99_999_999,
                },
                presence: Presence::Required,
                meaning: "The meter's identification number, eight decimal digits.",
                applies: Applies::Both,
            },
            Setting {
                name: "version",
                kind: BYTE,
                presence: Presence::Required,
                meaning: "The meter's version, as its link address carries it.",
                applies: Applies::Both,
            },
            Setting {
                name: "medium",
                kind: BYTE,
                presence: Presence::Required,
                meaning: "The meter's device type, EN 13757-3: 2 electricity, 3 gas, 7 water.",
                applies: Applies::Both,
            },
            Setting {
                name: "timeout",
                kind: Kind::Duration,
                presence: Presence::Default(Fixed::Duration(TIMEOUT)),
                meaning: "How long a meter that does not answer is waited on.",
                applies: Applies::Both,
            },
        ],
    };

    fn configured(address: &str, settings: &Read) -> Result<Self> {
        let manufacturer: [u8; 3] = settings
            .text("manufacturer")
            .as_bytes()
            .try_into()
            .ok()
            .filter(|letters: &[u8; 3]| letters.iter().all(u8::is_ascii_uppercase))
            .ok_or_else(|| protocol_error("a manufacturer that is not three capital letters"))?;
        let byte = |name| {
            u8::try_from(settings.integer(name))
                .map_err(|_| protocol_error(format!("a {name} over 255")))
        };
        let identity = Identity {
            ident: u32::try_from(settings.integer("ident"))
                .map_err(|_| protocol_error("an ident over eight digits"))?,
            manufacturer,
            version: byte("version")?,
            medium: byte("medium")?,
        };
        let link = identity.link_address();
        let air: Arc<dyn Line> = match address {
            "loopback" => Arc::new(loopback::LoopbackRadio::new(Meter::new(identity))),
            other => {
                return Err(protocol_error(format!(
                    "{other:?} is not an air this estate has; `loopback` is"
                )));
            }
        };
        Ok(Self::new(air, link).timing_out_after(settings.duration("timeout")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::LoopbackRadio;
    use xcore::settings::Given;

    #[test]
    fn wireless_m_bus_declares_its_settings_and_reads_through_them() {
        assert_eq!(
            WirelessMBusTransport::SETTINGS.problems(),
            Vec::<String>::new()
        );
        let mut given = vec![
            ("manufacturer".to_string(), Given::Text("ABC".to_string())),
            ("ident".to_string(), Given::Integer(1)),
            ("version".to_string(), Given::Integer(0)),
            ("medium".to_string(), Given::Integer(3)),
        ];
        let built = WirelessMBusTransport::open("loopback", Applies::Send, &given).expect("built");
        assert_eq!(built.address, identity().link_address());
        assert_eq!(built.timeout, TIMEOUT);
        given[0].1 = Given::Text("abc".to_string());
        assert!(WirelessMBusTransport::open("loopback", Applies::Send, &given).is_err());
        let Err(refused) = WirelessMBusTransport::open("loopback", Applies::Receive, &given[..2])
        else {
            panic!("version and medium are required");
        };
        assert!(refused.message.contains("medium"), "{}", refused.message);
    }

    fn identity() -> Identity {
        Identity {
            ident: 1,
            manufacturer: *b"ABC",
            version: 0,
            medium: 3,
        }
    }

    #[test]
    fn a_device_names_itself_and_the_meter_it_talks_to() {
        let device = WirelessMBusTransport::new(
            Arc::new(LoopbackRadio::new(Meter::new(identity()))),
            identity().link_address(),
        );
        assert_eq!(device.name(), "wireless-m-bus");
        assert!(device.claims().is_none());
        assert!(device.directions().receives() && device.directions().sends());
        assert_eq!(
            device.origin(&device.address),
            "wmbus://loopback/ABC-00000001"
        );
        assert!(device.listen().expect("quiet").is_none());
    }

    #[test]
    fn a_frame_that_is_not_the_meters_data_is_refused() {
        let frame = Frame {
            control: ACK,
            address: identity().link_address(),
            ci: CI_VARIABLE_SHORT,
            data: vec![1, 0, 0, 0],
        };
        acknowledged(&frame).expect("an acknowledgement");
        assert!(
            records_of(&frame, RSP_UD).is_err(),
            "an acknowledgement is no data"
        );
        let data = Frame {
            control: RSP_UD,
            ci: CI_DATA_SEND,
            ..frame
        };
        assert!(records_of(&data, RSP_UD).is_err(), "no short header");
        assert!(acknowledged(&data).is_err(), "data is no acknowledgement");
    }
}
