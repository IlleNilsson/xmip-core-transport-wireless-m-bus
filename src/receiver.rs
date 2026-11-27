//! A receiver stick on a serial port: the air a wireless M-Bus Location
//! speaks over where a stick is plugged in, through the host controller
//! interface of IMST's Wireless M-Bus Gateway firmware (`hci.rs`), the
//! port opened by the serial technology as every serial port is, its
//! messages SLIP frames (`serial::Framing::Slip`).
//!
//! The stick is set to listen in the Location's mode, handing the host
//! every telegram it hears, at the first use of its port and again after a
//! use failed, as a stick unplugged and plugged in again has forgotten it: its
//! active configuration is read, the link mode and the notification set in
//! it and the address filter taken off, and it is written back, the rest
//! as it was. A frame put on the line goes plain in one send request, the
//! port held until the stick's response; a frame taken off it is the next
//! telegram the Location's meter sent, in the form the air carries it, its
//! CRCs on, so the transport reads a stick as it reads the loopback radio.
//! A telegram of another meter is kept for the Location listening for it
//! (`heard.rs`).

use std::sync::Arc;
use std::time::Duration;
use xcore::clock::{Clock, SystemClock, between};

use serial::{Framing, SerialTransport};
use transport::error::{Result, protocol_error};
use transport::line::Line;
use transport::{Configured, Sharing, TransportError};

use crate::frame::Frame;
use crate::hci::{
    ADDRESS_FILTER, ANSWERED_WITHIN, BAUD, GATEWAY, GET_ACTIVE_CONFIGURATION, Message, Mode,
    NOTIFY_RECEIVED, SET_ACTIVE_CONFIGURATION,
};
use crate::heard::{Heard, Next};

/// A Location's side of a receiver stick on a serial port.
#[derive(Clone)]
pub struct Receiver {
    port: SerialTransport,
    /// The meter whose telegrams this side takes.
    meter: [u8; 8],
    mode: Mode,
    /// The transmit power, in dBm.
    power: i8,
    heard: Arc<Heard>,
}

impl Receiver {
    /// A stick on the serial port at `path`, listening for `meter` in
    /// `mode` and sending to it at `power` dBm.
    #[must_use]
    pub fn on(path: &str, meter: [u8; 8], mode: Mode, power: i8) -> Self {
        Self::over(SerialTransport::new(path, BAUD), meter, mode, power)
    }

    /// The same on `port`, the line opened as it is opened: a test's port.
    #[must_use]
    pub fn over(port: SerialTransport, meter: [u8; 8], mode: Mode, power: i8) -> Self {
        let heard = Arc::new(Heard::default());
        heard.listen_for(meter);
        Self {
            port: port.framed(Framing::Slip),
            meter,
            mode,
            power,
            heard,
        }
    }

    /// `request` asked of the stick and its response, the port held until
    /// it came; every telegram heard meanwhile kept.
    fn asked(&self, request: &Message) -> Result<Message> {
        let mut held = self.port.exchanging();
        held.transmit(&Framing::Slip, &request.encode()?)?;
        let began = SystemClock.monotonic_nanos();
        loop {
            let left = ANSWERED_WITHIN.saturating_sub(since(began));
            let Some(bytes) = held.receive(&Framing::Slip, left)? else {
                return Err(TransportError::retryable(format!(
                    "the receiver did not answer request {:#04x} of service {:#04x}",
                    request.id, request.sap
                )));
            };
            let message = Message::decode(&bytes)?;
            if message.answers(request) {
                return Ok(message);
            }
            self.kept(&message)?;
        }
    }

    /// The telegram `message` says the stick heard, kept for the Location
    /// listening for its meter; any other event let go.
    fn kept(&self, message: &Message) -> Result<()> {
        if let Some(frame) = message.heard()? {
            self.heard.keep(frame.address, frame.encode()?);
        }
        Ok(())
    }

    /// The stick listening in this Location's mode and handing the host
    /// every telegram, unless it was set to already.
    fn listening(&self) -> Result<()> {
        let link = self.mode.listening();
        if self.heard.listening() == Some(link) {
            return Ok(());
        }
        let read = Message::new(GATEWAY, GET_ACTIVE_CONFIGURATION, Vec::new());
        let mut configuration = self.asked(&read)?.granted()?.to_vec();
        let [mode, low, high, ..] = configuration.as_mut_slice() else {
            return Err(protocol_error("a configuration too short for its options"));
        };
        let options = u16::from_le_bytes([*low, *high]);
        let options = (options | NOTIFY_RECEIVED) & !ADDRESS_FILTER;
        [*low, *high] = options.to_le_bytes();
        *mode = link;
        let write = Message::new(GATEWAY, SET_ACTIVE_CONFIGURATION, configuration);
        self.asked(&write)?.granted()?;
        self.heard.listens(Some(link));
        Ok(())
    }

    /// `outcome`, the stick to be set again at the next use where it failed.
    fn minded<T>(&self, outcome: Result<T>) -> Result<T> {
        if outcome.is_err() {
            self.heard.listens(None);
        }
        outcome
    }

    /// The next telegram of this Location's meter within `timeout`: one
    /// kept, or one read off the port where no other receive reads it,
    /// every other kept for its own Location. The port read with nothing
    /// within the wait, what was kept meanwhile is looked at once more.
    fn next(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let began = SystemClock.monotonic_nanos();
        loop {
            let left = timeout.saturating_sub(since(began));
            let turn = match self.heard.next(self.meter, left) {
                Next::Heard(frame) => return Ok(Some(frame)),
                Next::Quiet => return Ok(None),
                Next::Read(turn) => turn,
            };
            let read = if left.is_zero() {
                None
            } else {
                Line::receive(&self.port, left)?
            };
            let Some(bytes) = read else {
                drop(turn);
                return Ok(match self.heard.next(self.meter, Duration::ZERO) {
                    Next::Heard(frame) => Some(frame),
                    Next::Read(_) | Next::Quiet => None,
                });
            };
            self.kept(&Message::decode(&bytes)?)?;
        }
    }
}

/// How long since `began`, a monotonic reading of the system's clock: what
/// a wait on the stick has left is counted on it, as serial's receive does.
pub(crate) fn since(began: i128) -> Duration {
    between(began, SystemClock.monotonic_nanos())
}

/// A stick is the air: what is transmitted goes out through it, and what is
/// received is what it heard from the Location's meter.
impl Line for Receiver {
    fn name(&self) -> String {
        self.port.port().to_string()
    }

    /// `frame`, as the air carries it, sent plain: what was kept of the
    /// meter's before it is no answer to it, and is let go.
    fn transmit(&self, frame: &[u8]) -> Result<()> {
        let frame = Frame::decode(frame)?;
        let sent = self.listening().and_then(|()| {
            self.heard.forget(self.meter);
            let request = Message::send(&frame, self.mode.sending(), self.power)?;
            self.asked(&request)?.granted().map(drop)
        });
        self.minded(sent)
    }

    fn receive(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let heard = self.listening().and_then(|()| self.next(timeout));
        self.minded(heard)
    }

    /// This side on the stick's one port on the node, and on what the
    /// node's Locations on it heard, kept under the port's path.
    fn shared_on(&self, sharing: &Arc<dyn Sharing>) -> Option<Arc<dyn Line>> {
        let port = Configured::shared_by(self.port.clone(), sharing);
        let heard = sharing.one(port.port(), Heard::default);
        heard.listen_for(self.meter);
        Some(Arc::new(Self {
            port,
            heard,
            ..self.clone()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{BufReader, ErrorKind, Read, Write};
    use std::sync::Mutex;
    use xcore::lock::Locked;

    use crate::frame::SND_NR;
    use crate::hci::{NOTIFY_RECEIVED, OK, SEND};
    use crate::stick::Stick;
    use crate::{Identity, Meter, WirelessMBusTransport};
    use m_bus::record::CI_VARIABLE_SHORT;
    use serial::{Opens, Port, Watch};
    use transport::Transport;

    /// A stick on a port in process: each SLIP frame the host writes
    /// answered at once as the stick answers it, into what the host reads
    /// next; a read with nothing there times out at once, as a port does.
    #[derive(Clone)]
    struct OnThePort {
        stick: Arc<Stick>,
        to_host: Arc<Mutex<VecDeque<u8>>>,
        /// Each request's access point and identifier, in order.
        asked: Arc<Mutex<Vec<(u8, u8)>>>,
        /// Whether the stick has been unplugged.
        gone: Arc<Mutex<bool>>,
    }

    impl OnThePort {
        fn with(identity: Identity) -> Self {
            Self {
                stick: Arc::new(Stick::new(Meter::new(identity))),
                to_host: Arc::default(),
                asked: Arc::default(),
                gone: Arc::default(),
            }
        }

        /// The serial line its host opens, `COM7`.
        fn line(&self) -> SerialTransport {
            SerialTransport::new("COM7", BAUD).opened_by(Arc::new(self.clone()))
        }

        /// The stick puts `bytes` on its line unasked.
        fn sends(&self, bytes: &[u8]) {
            self.to_host.locked().extend(bytes);
        }

        fn asked(&self) -> Vec<(u8, u8)> {
            self.asked.locked().clone()
        }
    }

    impl Opens for OnThePort {
        fn open(&self, _port: &str, _baud: u32) -> Result<Box<dyn Port>> {
            Ok(Box::new(self.clone()))
        }
    }

    impl Read for OnThePort {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.to_host.locked().read(buf)? {
                0 => Err(ErrorKind::TimedOut.into()),
                read => Ok(read),
            }
        }
    }

    impl Write for OnThePort {
        /// One whole SLIP frame, as a host's exchange writes it at once.
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if *self.gone.locked() {
                return Ok(buf.len());
            }
            let message = serial::read_frame(&mut BufReader::new(buf), &Framing::Slip)
                .and_then(|message| {
                    let request = Message::decode(&message)?;
                    self.asked.locked().push((request.sap, request.id));
                    self.stick.answer(&message)
                })
                .map_err(std::io::Error::other)?;
            self.sends(&message.unwrap_or_default());
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Port for OnThePort {
        fn wait(&mut self, _wait: Duration) -> std::io::Result<()> {
            Ok(())
        }

        fn speed(&mut self, _baud: u32) -> std::io::Result<()> {
            Ok(())
        }

        fn watch(&self) -> std::io::Result<Arc<dyn Watch>> {
            Ok(Arc::new(self.clone()))
        }
    }

    impl Watch for OnThePort {
        fn readable(&self, _wait: Duration) -> std::io::Result<bool> {
            Ok(!self.to_host.locked().is_empty())
        }

        fn cancel(&self) {}
    }

    fn identity(ident: u32) -> Identity {
        Identity {
            ident,
            manufacturer: *b"XMP",
            version: 1,
            medium: 7,
        }
    }

    /// A device on `port`'s stick, talking to the meter `identity` names.
    fn device(port: &OnThePort, identity: &Identity) -> WirelessMBusTransport {
        let link = identity.link_address();
        let receiver = Receiver::over(port.line(), link, Mode::T, 10);
        WirelessMBusTransport::new(Arc::new(receiver), link)
    }

    /// The first use sets the stick listening, every telegram handed over
    /// and the address filter off, its LED and calibration as they were,
    /// and nothing sets it again; a Stream crosses to the meter and back
    /// through it in as many telegrams as it takes.
    #[test]
    fn a_location_sets_its_stick_listening_once_and_writes_and_reads_its_meter() {
        let port = OnThePort::with(identity(12_345_678));
        let device = device(&port, &identity(12_345_678));
        device.initialize().expect("SND_NKE acknowledged");
        let configuration = port.stick.configuration();
        assert_eq!(configuration[0], Mode::T.listening());
        assert_eq!(
            u16::from_le_bytes([configuration[1], configuration[2]]),
            NOTIFY_RECEIVED
        );
        assert_eq!(
            &configuration[3..],
            [0, 0, 50, 0, 0x60, 0xea, 0, 0],
            "as they were"
        );
        let long: Vec<u8> = (0..3000u32)
            .map(|n| u8::try_from(n % 251).unwrap_or(0))
            .collect();
        device.send("", &long).expect("thirteen telegrams");
        assert_eq!(port.stick.meter().held(), long);
        let read = device.read_stream().expect("read back");
        assert_eq!(read.bytes, long);
        assert_eq!(read.origin_uri, "wmbus://COM7/XMP-12345678");
        let asked = port.asked();
        let configured = [
            (GATEWAY, GET_ACTIVE_CONFIGURATION),
            (GATEWAY, SET_ACTIVE_CONFIGURATION),
        ];
        assert_eq!(asked[..2], configured);
        assert!(
            asked[2..].iter().all(|&asked| asked == (GATEWAY, SEND)),
            "{asked:?}"
        );
    }

    /// Two Locations of a node on one stick, each its own meter: the one
    /// whose receive reads the port keeps the other's telegram for it, and
    /// the stick is set listening once for both.
    #[test]
    fn a_telegram_of_another_meter_is_kept_for_the_location_listening_for_it() {
        let node: Arc<dyn Sharing> = Arc::new(transport::Shared::default());
        let port = OnThePort::with(identity(1));
        let on = |ident| {
            let link = identity(ident).link_address();
            Receiver::over(port.line(), link, Mode::T, 10)
                .shared_on(&node)
                .expect("a stick's port is shared")
        };
        let (first, second) = (on(1), on(2));
        assert_eq!(first.receive(Duration::ZERO).expect("quiet"), None);
        let unasked = Frame {
            control: SND_NR,
            address: identity(2).link_address(),
            ci: CI_VARIABLE_SHORT,
            data: vec![1, 0, 0, 0, 0x0f],
        };
        port.sends(
            &port
                .stick
                .hears(&unasked)
                .expect("heard")
                .expect("handed over"),
        );
        let quiet = first.receive(Duration::from_millis(250)).expect("read");
        assert_eq!(quiet, None, "the other meter's");
        let kept = second
            .receive(Duration::ZERO)
            .expect("kept")
            .expect("a telegram");
        assert_eq!(Frame::decode(&kept).expect("the air's form"), unasked);
        assert_eq!(port.asked().len(), 2, "set once: {:?}", port.asked());
    }

    /// A stick that does not answer is worth asking again, and is set
    /// listening again at the next use; a request it does not carry out is
    /// refused in the specification's words.
    #[test]
    fn a_silent_stick_is_asked_again_and_a_refusal_is_said() {
        let port = OnThePort::with(identity(1));
        let receiver = Receiver::over(port.line(), identity(1).link_address(), Mode::C, 10);
        receiver.receive(Duration::ZERO).expect("set listening");
        *port.gone.locked() = true;
        let air: &dyn Line = &receiver;
        let error = air.transmit(
            &Frame {
                control: crate::frame::SND_NKE,
                address: identity(1).link_address(),
                ci: 0x51,
                data: Vec::new(),
            }
            .encode()
            .expect("encode"),
        );
        let error = error.expect_err("no answer");
        assert!(
            error.retryable && error.message.contains("did not answer"),
            "{error}"
        );
        *port.gone.locked() = false;
        receiver.receive(Duration::ZERO).expect("set again");
        assert_eq!(port.asked().len(), 4, "set twice: {:?}", port.asked());
        let unknown = Message::new(GATEWAY, 0x41, Vec::new());
        let refused = receiver
            .asked(&unknown)
            .expect("answered")
            .granted()
            .map(<[u8]>::to_vec);
        let refused = refused.expect_err("refused");
        assert!(
            refused.message.contains("command not supported"),
            "{refused}"
        );
        let ping = Message::new(crate::hci::DEVICE_MANAGEMENT, crate::hci::PING, Vec::new());
        assert_eq!(receiver.asked(&ping).expect("pong").payload, [OK]);
    }

    /// The stick in process on the device's end of a pseudo-terminal, the
    /// host opening its line's end as it opens a stick's port: a Stream
    /// written through it reaches the meter whole, and the host reads the
    /// meter's acknowledgement, the pair held open until it has.
    #[cfg(unix)]
    #[test]
    fn a_stick_on_a_pseudo_terminal_takes_a_stream_to_its_meter() {
        let mut terminal =
            serial::terminal::Terminal::pair(Duration::from_secs(10)).expect("a pair");
        let link = identity(7).link_address();
        let receiver = Receiver::on(&terminal.path(), link, Mode::T, 10);
        let stick = Stick::new(Meter::new(identity(7)));
        std::thread::scope(|scope| {
            let device = WirelessMBusTransport::new(Arc::new(receiver), link);
            let sent = scope.spawn(move || device.send("", b"<probe/>"));
            while stick.meter().held().is_empty() {
                let message = terminal
                    .read_frame(&Framing::Slip)
                    .expect("the host writes");
                let answer = stick.answer(&message).expect("answered");
                terminal
                    .write(&answer.unwrap_or_default())
                    .expect("the stick answers");
            }
            let sent = sent.join().expect("joined");
            sent.expect("sent through the stick, acknowledged");
        });
        assert_eq!(stick.meter().held(), b"<probe/>");
    }
}
