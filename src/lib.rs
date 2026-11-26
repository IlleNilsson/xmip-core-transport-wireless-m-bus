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
//!
//! **Built with its `declaration` feature alone** the crate is
//! [`declaration`], what a Location of it is configured with, and none of
//! the code above, which is behind the default feature: what the surfaces'
//! library carries (ADR-0064, amendment 2026-11-26).

#[cfg(feature = "declaration")]
pub mod declaration;
#[cfg(feature = "default")]
pub mod frame;
#[cfg(feature = "default")]
pub mod loopback;

#[cfg(feature = "default")]
use std::sync::Arc;
#[cfg(feature = "default")]
use std::time::Duration;

#[cfg(feature = "default")]
use context::property::M_BUS_IDENTIFICATION_NUMBER;
#[cfg(feature = "default")]
use m_bus::record::{self, CI_DATA_SEND, CI_VARIABLE_SHORT};
#[cfg(feature = "default")]
use transport::error::{Result, protocol_error};
#[cfg(feature = "default")]
use transport::{Acknowledgement, Arrived, Configured, Directions, Taken, Transport};
#[cfg(feature = "default")]
use xcore::settings::{Read, Settings};

/// Why a Stream a meter sends unasked cannot be acknowledged after the
/// receive cycle.
#[cfg(feature = "default")]
pub const AT_MOST_ONCE: &str = "a wireless M-Bus meter sends SND_NR unasked and waits for no \
                                reply: the telegram is off the air as it is heard";

#[cfg(feature = "declaration")]
pub use declaration::TIMEOUT;
#[cfg(feature = "default")]
pub use frame::{Frame, MAX_DATA};
#[cfg(feature = "default")]
pub use m_bus::{Identity, Meter};
#[cfg(feature = "default")]
use transport::line::Line;

#[cfg(feature = "default")]
use crate::frame::{ACK, REQ_UD2, RSP_UD, SND_NKE, SND_NR, SND_UD};

/// The room a meter's telegram leaves for records: the frame's data less
/// the short header.
#[cfg(feature = "default")]
pub const ANSWER_ROOM: usize = MAX_DATA - 4;

/// The other device's side of the air: a concentrator, a gateway, a
/// hand-held reader.
#[cfg(feature = "default")]
#[derive(Clone)]
pub struct WirelessMBusTransport {
    air: Arc<dyn Line>,
    address: [u8; 8],
    timeout: Duration,
}

#[cfg(feature = "default")]
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

    /// `SND_NKE`: initialize the meter.
    ///
    /// # Errors
    /// No acknowledgement.
    pub fn initialize(&self) -> Result<()> {
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
                return Ok(Taken::new(self.origin(&answer.address), bytes)
                    .observing(M_BUS_IDENTIFICATION_NUMBER, frame::label(&answer.address)));
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
                    .observing(M_BUS_IDENTIFICATION_NUMBER, frame::label(&address))
                }));
            };
            let frame = Frame::decode(&heard)?;
            let (chunk, more) = records_of(&frame, SND_NR)?;
            bytes.extend_from_slice(&chunk);
            from = Some(frame.address);
            if !more {
                return Ok(Some(
                    Taken::new(
                        format!("{}?unsolicited=true", self.origin(&frame.address)),
                        bytes,
                    )
                    .observing(M_BUS_IDENTIFICATION_NUMBER, frame::label(&frame.address)),
                ));
            }
        }
    }
}

/// The records a meter's frame carries past its short header, if it is
/// `control`.
#[cfg(feature = "default")]
fn records_of(frame: &Frame, control: u8) -> Result<(Vec<u8>, bool)> {
    if frame.control != control || frame.ci != CI_VARIABLE_SHORT {
        return Err(protocol_error(format!(
            "control {:#04x} with information {:#04x} is not the meter's data",
            frame.control, frame.ci
        )));
    }
    record::unpack(frame.data.get(4..).unwrap_or(&[]))
}

#[cfg(feature = "default")]
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

#[cfg(feature = "default")]
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

    fn waiting(&self) -> transport::Waiting {
        transport::Waiting::Waits("the air is heard until its timeout, then the meter is read")
    }

    /// What the meter sends unasked if it has; the Stream it holds
    /// otherwise. Each arrives whole. Sent unasked, `SND_NR` has no reply:
    /// acceptance is at-most-once ([`AT_MOST_ONCE`]). Read by `REQ_UD2`, the
    /// verdict has nothing to tell the meter, whichever it is: the read
    /// consumes nothing, so a cycle that did not complete loses nothing —
    /// the next read finds the Stream again.
    fn receive_up_to(&self, _most: usize) -> Result<Vec<Arrived>> {
        let (taken, acknowledgement) = match self.listen()? {
            Some(heard) => (heard, Acknowledgement::at_most_once(AT_MOST_ONCE)),
            None => (self.read_stream()?, Acknowledgement::unconsumed()),
        };
        Ok(vec![
            Arrived::whole(taken.origin_uri, taken.bytes, acknowledgement)
                .observing_all(taken.observed),
        ])
    }

    fn sending(&self) -> transport::Sending {
        transport::Sending::Gathered("one wM-Bus telegram carries the whole body")
    }

    /// Write to the meter this device is configured for.
    fn send_from(&self, _target: &str, outgoing: transport::Outgoing<'_>) -> Result<()> {
        let bytes: &[u8] = &outgoing.gather()?;
        self.write_stream(bytes)
    }
}

#[cfg(feature = "default")]
impl Configured for WirelessMBusTransport {
    const SETTINGS: &'static Settings = declaration::SETTINGS;
    const SCHEMES: &'static [&'static str] = declaration::SCHEMES;

    fn timeout_kept(&mut self) -> Option<&mut dyn transport::configured::TimeoutKept> {
        Some(&mut self.timeout)
    }

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
        Ok(Self::new(air, link))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::LoopbackRadio;
    use xcore::settings::{Applies, Given};

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
