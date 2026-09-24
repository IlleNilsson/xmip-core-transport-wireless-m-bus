//! A meter on an in-process radio, and both ends of one wireless M-Bus
//! exchange on it (ADR-0051).
//!
//! [`LoopbackRadio`] is the air every test and every box without a
//! receiver drives, the way can-bus drives its loopback bus: what the
//! device transmits, the meter hears and answers, and the answer is what
//! the device receives next. The loopback pair is a device on that air
//! and the meter it writes a Stream to; the far end is the meter holding
//! it, read back a telegram at a time. One air, one thread: the round goes
//! in order.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use m_bus::record::{CI_DATA_SEND, CI_VARIABLE_SHORT};
use m_bus::{Identity, Meter};
use transport::error::Result;
use transport::line::Line;
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
use transport::{Arrived, Transport};

use crate::frame::{ACK, BROADCAST, Frame, REQ_UD2, RSP_UD, SND_NKE, SND_NR, SND_UD};
use crate::{ANSWER_ROOM, WirelessMBusTransport};

/// A meter listening on an in-process radio.
pub struct LoopbackRadio {
    meter: Arc<Meter>,
    to_device: Mutex<VecDeque<Vec<u8>>>,
}

impl LoopbackRadio {
    #[must_use]
    pub fn new(meter: Meter) -> Self {
        Self {
            meter: Arc::new(meter),
            to_device: Mutex::new(VecDeque::new()),
        }
    }

    #[must_use]
    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    /// The meter transmits the Stream it holds unasked, as many `SND_NR`
    /// telegrams as it takes.
    ///
    /// # Errors
    /// Never on this air; the signature is the line's.
    pub fn send_unasked(&self) -> Result<()> {
        loop {
            let data = self.meter.give(ANSWER_ROOM)?;
            let more = data.last() == Some(&m_bus::record::MORE_FOLLOW);
            self.queue(self.meter_frame(SND_NR, data)?);
            if !more {
                return Ok(());
            }
        }
    }

    /// A frame from the meter: its address, the short header, then `data`.
    fn meter_frame(&self, control: u8, data: Vec<u8>) -> Result<Vec<u8>> {
        let mut body = Identity::short_header(self.meter.next_access()).to_vec();
        body.extend(data);
        Frame {
            control,
            address: self.meter.identity().link_address(),
            ci: CI_VARIABLE_SHORT,
            data: body,
        }
        .encode()
    }

    /// What the meter answers `frame` with, if it is for the meter.
    fn answer(&self, frame: &Frame) -> Result<Option<Vec<u8>>> {
        let mine = self.meter.identity().link_address();
        if frame.address != mine && frame.address != BROADCAST {
            return Ok(None);
        }
        match (frame.control, frame.ci) {
            (SND_NKE, _) => {
                self.meter.reset();
                Ok(Some(self.meter_frame(ACK, Vec::new())?))
            }
            (SND_UD, CI_DATA_SEND) => {
                self.meter.take(&frame.data)?;
                Ok(Some(self.meter_frame(ACK, Vec::new())?))
            }
            (REQ_UD2, _) => {
                let data = self.meter.give(ANSWER_ROOM)?;
                Ok(Some(self.meter_frame(RSP_UD, data)?))
            }
            _ => Ok(None),
        }
    }

    fn queue(&self, bytes: Vec<u8>) {
        self.to_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(bytes);
    }
}

impl Line for LoopbackRadio {
    fn name(&self) -> String {
        "loopback".to_string()
    }

    fn transmit(&self, bytes: &[u8]) -> Result<()> {
        if let Some(answer) = self.answer(&Frame::decode(bytes)?)? {
            self.queue(answer);
        }
        Ok(())
    }

    fn receive(&self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        Ok(self
            .to_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front())
    }
}

impl WirelessMBusTransport {
    /// Both ends on one air: a device and the meter that answers it, on a
    /// fresh [`LoopbackRadio`], the loopback timeout on the device.
    #[must_use]
    pub fn loopback() -> Self {
        let identity = Identity {
            ident: 12_345_678,
            manufacturer: *b"XMP",
            version: 1,
            medium: 7,
        };
        let address = identity.link_address();
        Self::new(Arc::new(LoopbackRadio::new(Meter::new(identity))), address)
            .timing_out_after(LOOPBACK_TIMEOUT)
    }
}

/// The meter on the air, holding what the device wrote until it is read
/// back.
struct Holding {
    device: WirelessMBusTransport,
    address: String,
}

impl FarEnd for Holding {
    fn address(&self) -> &str {
        &self.address
    }

    fn take_one(self: Box<Self>) -> Result<Arrived> {
        self.device.read_stream()
    }
}

impl Loopback for WirelessMBusTransport {
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        Ok(Box::new(Holding {
            device: self.clone(),
            address: self.origin(&self.address),
        }))
    }

    /// A fresh device on the same air writes to the meter.
    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        Self::new(Arc::clone(&self.air), self.address)
            .timing_out_after(self.timeout)
            .send(address, payload)
    }

    fn unblock(&self, _address: &str) {
        // The air is in-process; nothing listens on a socket.
    }

    /// In order on one thread: the air has one device asking, so the write
    /// goes first and the read-back finds what it left.
    fn round(&self, payload: &[u8]) -> Result<Arrived> {
        self.round_in_order(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport::payload::edge_payloads;

    #[test]
    fn the_loopback_returns_the_edges_whole() {
        let loopback = WirelessMBusTransport::loopback();
        for (name, bytes) in edge_payloads() {
            let arrived = loopback
                .round(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(arrived.bytes, bytes, "{name}");
            assert_eq!(
                arrived.origin_uri, "wmbus://loopback/XMP-12345678",
                "{name}"
            );
        }
        assert!(
            loopback.ceiling().is_none(),
            "as many telegrams as it takes"
        );
        assert!(loopback.refuses(b"x").is_none());
    }

    #[test]
    fn a_long_stream_crosses_in_many_telegrams_and_the_meter_sends_it_unasked() {
        let radio = Arc::new(LoopbackRadio::new(Meter::new(Identity {
            ident: 42,
            manufacturer: *b"ABC",
            version: 2,
            medium: 2,
        })));
        let device = WirelessMBusTransport::new(
            Arc::clone(&radio) as Arc<dyn Line>,
            radio.meter().identity().link_address(),
        )
        .timing_out_after(Duration::from_millis(10));
        let long: Vec<u8> = (0..3000u32)
            .map(|n| u8::try_from(n % 253).unwrap_or(0))
            .collect();
        device.initialise().expect("SND_NKE");
        device.send("", &long).expect("thirteen telegrams");
        assert_eq!(radio.meter().held(), long);
        let read = device.receive().expect("asked");
        assert_eq!(read[0].bytes, long);
        assert_eq!(read[0].origin_uri, "wmbus://loopback/ABC-00000042");
        radio.send_unasked().expect("SND_NR");
        let heard = device.receive().expect("unasked");
        assert_eq!(heard[0].bytes, long);
        assert_eq!(
            heard[0].origin_uri,
            "wmbus://loopback/ABC-00000042?unsolicited=true"
        );
    }

    #[test]
    fn a_meter_that_is_not_addressed_does_not_answer_and_a_broadcast_reaches_it() {
        let radio = Arc::new(LoopbackRadio::new(Meter::new(Identity {
            ident: 7,
            manufacturer: *b"XYZ",
            version: 0,
            medium: 7,
        })));
        let elsewhere = WirelessMBusTransport::new(Arc::clone(&radio) as Arc<dyn Line>, [0; 8])
            .timing_out_after(Duration::from_millis(10));
        let error = elsewhere.initialise().expect_err("silence");
        assert!(error.retryable, "a meter may be out of range");
        let everyone = WirelessMBusTransport::new(Arc::clone(&radio) as Arc<dyn Line>, BROADCAST)
            .timing_out_after(Duration::from_millis(10));
        everyone
            .write_stream(b"to whom it may concern")
            .expect("broadcast");
        assert_eq!(radio.meter().held(), b"to whom it may concern");
        let unasked = Frame {
            control: SND_NR,
            address: [1; 8],
            ci: CI_DATA_SEND,
            data: vec![],
        };
        radio.queue(unasked.encode().expect("encode"));
        assert!(elsewhere.listen().is_err(), "no short header is no data");
    }
}
