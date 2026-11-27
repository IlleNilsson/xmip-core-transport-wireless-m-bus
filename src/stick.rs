//! A receiver stick in process, a meter on its air: what IMST's Wireless
//! M-Bus Gateway firmware puts on its serial line for each message its host
//! writes (`hci.rs`), the meter heard and answering on a [`LoopbackRadio`]
//! (ADR-0051). It is the far end a node's Location reaches through a
//! stick's port where no stick is plugged in: the device's end of a
//! pseudo-terminal answering as the stick does, in the root's service test
//! and this crate's, and a port in process in this crate's tests.
//!
//! As the firmware does: a ping is answered; the active configuration is
//! read and set; a send request is answered, its telegram goes on the air,
//! the telegram sent is told where the configuration asks for that, and
//! the meter's answer is handed to the host as heard where the
//! configuration listens and hands heard telegrams over. Any other request
//! is answered "command not supported".

use std::sync::Mutex;
use std::time::Duration;
use xcore::lock::Locked;

use m_bus::Meter;
use serial::Framing;
use transport::error::Result;
use transport::line::Line;

use crate::frame::Frame;
use crate::hci::{
    DEVICE_MANAGEMENT, GATEWAY, GET_ACTIVE_CONFIGURATION, Message, Mode, NOTIFY_RECEIVED,
    NOTIFY_SENT, OFF, OK, PING, SEND, SET_ACTIVE_CONFIGURATION, TRANSMITTED, UNSUPPORTED,
};
use crate::loopback::LoopbackRadio;

/// The active configuration a stick starts with here: the receiver off,
/// nothing handed to the host, its LED flashing for 50 ms, its radio
/// calibrated again each minute.
const STARTED: [u8; 11] = [OFF, 0, 0, 0, 0, 50, 0, 0x60, 0xea, 0, 0];

/// The signal strength every telegram is heard at here, in dBm.
const SIGNAL: i8 = -60;

/// A receiver stick with a meter in range.
pub struct Stick {
    radio: LoopbackRadio,
    configuration: Mutex<Vec<u8>>,
}

impl Stick {
    /// A stick as it starts, `meter` on its air.
    #[must_use]
    pub fn new(meter: Meter) -> Self {
        Self {
            radio: LoopbackRadio::new(meter),
            configuration: Mutex::new(STARTED.to_vec()),
        }
    }

    #[must_use]
    pub fn meter(&self) -> &Meter {
        self.radio.meter()
    }

    /// The active configuration, as the host last set it.
    #[must_use]
    pub fn configuration(&self) -> Vec<u8> {
        self.configuration.locked().clone()
    }

    /// What the stick puts on its line for `message`, a message its host
    /// wrote, SLIP's ends taken off: each message it answers with, SLIP
    /// framed, one after the other.
    ///
    /// # Errors
    /// A message that is none, or a send request whose telegram is none.
    pub fn answer(&self, message: &[u8]) -> Result<Option<Vec<u8>>> {
        let request = Message::decode(message)?;
        let answers = match (request.sap, request.id) {
            (DEVICE_MANAGEMENT, PING) => vec![Message::response(&request, OK, &[])],
            (GATEWAY, GET_ACTIVE_CONFIGURATION) => {
                vec![Message::response(&request, OK, &self.configuration())]
            }
            (GATEWAY, SET_ACTIVE_CONFIGURATION) => {
                self.configuration.locked().clone_from(&request.payload);
                vec![Message::response(&request, OK, &[])]
            }
            (GATEWAY, SEND) => self.sent(&request)?,
            _ => vec![Message::response(&request, UNSUPPORTED, &[])],
        };
        Self::framed(&answers)
    }

    /// What the stick puts on its line for `frame`, a telegram it heard
    /// unasked: the event, where its configuration hands heard telegrams
    /// over.
    ///
    /// # Errors
    /// A frame over a telegram's data.
    pub fn hears(&self, frame: &Frame) -> Result<Option<Vec<u8>>> {
        let heard = self.heard(frame)?;
        Self::framed(&heard.into_iter().collect::<Vec<_>>())
    }

    /// The answers to a send request: its response, the telegram sent told
    /// where that is asked for, and whatever the meter answered, heard.
    fn sent(&self, request: &Message) -> Result<Vec<Message>> {
        let frame = request.sent()?;
        let mut answers = vec![Message::response(request, OK, &[])];
        if self.options() & NOTIFY_SENT != 0 {
            answers.push(Message::new(GATEWAY, TRANSMITTED, vec![0; 5]));
        }
        self.radio.transmit(&frame.encode()?)?;
        while let Some(answer) = self.radio.receive(Duration::ZERO)? {
            answers.extend(self.heard(&Frame::decode(&answer)?)?);
        }
        Ok(answers)
    }

    /// The event of `frame` heard, where the stick listens and hands heard
    /// telegrams over.
    fn heard(&self, frame: &Frame) -> Result<Option<Message>> {
        let link = self.configuration.locked().first().copied().unwrap_or(OFF);
        if link == OFF || self.options() & NOTIFY_RECEIVED == 0 {
            return Ok(None);
        }
        let mode = if link == Mode::S.listening() {
            Mode::S
        } else {
            Mode::T
        };
        Message::received(frame, mode.heard(), SIGNAL).map(Some)
    }

    /// The option bits of the active configuration.
    fn options(&self) -> u16 {
        match self.configuration.locked().as_slice() {
            [_, low, high, ..] => u16::from_le_bytes([*low, *high]),
            _ => 0,
        }
    }

    /// `messages` on the line, each a SLIP frame; `None` where there are
    /// none.
    fn framed(messages: &[Message]) -> Result<Option<Vec<u8>>> {
        let mut line = Vec::new();
        for message in messages {
            line.extend(Framing::Slip.frame(&message.encode()?)?);
        }
        Ok(Some(line).filter(|line| !line.is_empty()))
    }
}
