//! What a receiver stick heard, kept for the Location listening for each
//! meter. A stick hears every meter in range, so where several Locations of
//! a node name its port, each its own meter, the telegram one Location's
//! receive takes off the port is kept for the Location whose meter sent it,
//! and that Location's waiting receive is told at once. One receive reads
//! the port at a time; the others wait to be told what it read, holding
//! nothing, as the serial technology's receives wait on a port's watch, and
//! the next takes the reading over once it lets go. A telegram from a meter
//! no Location listens for is let go, unless one listens for every meter
//! at the broadcast address.
//!
//! Kept with the port, under its path, in what the node's Locations share,
//! as wired M-Bus keeps its frame count bits; a receiver no node shares
//! keeps its own. The link mode the stick was set to listen in is kept with
//! it, so the stick is set once for all of them.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;
use xcore::lock::Locked;
use xcore::wake::Wake;

use crate::frame::BROADCAST;

/// How many telegrams of one meter are kept for its Location, the oldest
/// let go first: a meter sends every few seconds at the most, and its
/// Receive Location takes what was kept at its next receive.
const KEPT: usize = 16;

/// What one stick heard, and who reads its port.
#[derive(Default)]
pub(crate) struct Heard {
    kept: Mutex<Kept>,
    /// Told when a telegram is kept and when a receive lets the port go.
    told: Wake,
}

#[derive(Default)]
struct Kept {
    /// The telegrams heard and not yet taken, as the air carries them, by
    /// the meter that sent them, for each meter a Location listens for.
    by_meter: BTreeMap<[u8; 8], VecDeque<Vec<u8>>>,
    /// Whether a receive reads the port now.
    reading: bool,
    /// The link mode the stick listens in, once it was set to.
    listening: Option<u8>,
}

/// What a receive does next.
pub(crate) enum Next<'a> {
    /// Takes this telegram of its meter's.
    Heard(Vec<u8>),
    /// Reads the port, its turn given back once this is dropped.
    Read(Reading<'a>),
    /// Takes nothing: its wait ran out.
    Quiet,
}

/// A receive's turn to read the port.
pub(crate) struct Reading<'a>(&'a Heard);

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        self.0.kept.locked().reading = false;
        self.0.told.notify_all();
    }
}

impl Heard {
    /// A Location listens for `meter`'s telegrams.
    pub(crate) fn listen_for(&self, meter: [u8; 8]) {
        self.kept.locked().by_meter.entry(meter).or_default();
    }

    /// `frame` heard from `from`, kept for the Location listening for it,
    /// or for every meter, and that Location told; let go where none is.
    pub(crate) fn keep(&self, from: [u8; 8], frame: Vec<u8>) {
        let mut kept = self.kept.locked();
        let key = if kept.by_meter.contains_key(&from) {
            from
        } else {
            BROADCAST
        };
        if let Some(telegrams) = kept.by_meter.get_mut(&key) {
            if telegrams.len() == KEPT {
                telegrams.pop_front();
            }
            telegrams.push_back(frame);
            self.told.notify_all();
        }
    }

    /// Every telegram kept of `meter`'s let go: what was heard before a
    /// request to it is no answer to the request.
    pub(crate) fn forget(&self, meter: [u8; 8]) {
        if let Some(telegrams) = self.kept.locked().by_meter.get_mut(&meter) {
            telegrams.clear();
        }
    }

    /// What a receive for `meter` does within `left`: take a telegram kept
    /// of the meter's, read the port where no other receive does, or wait,
    /// holding nothing, until one is kept or the reading is let go. The
    /// wait ends holding the lock it decided under, so what ended it is
    /// still so.
    pub(crate) fn next(&self, meter: [u8; 8], left: Duration) -> Next<'_> {
        let kept = self.kept.locked();
        let (mut kept, ran_out) = self
            .told
            .wait_timeout_while(&self.kept, kept, left, |kept| {
                kept.reading && kept.by_meter.get(&meter).is_none_or(VecDeque::is_empty)
            });
        if let Some(frame) = kept.by_meter.get_mut(&meter).and_then(VecDeque::pop_front) {
            return Next::Heard(frame);
        }
        if ran_out || kept.reading {
            return Next::Quiet;
        }
        kept.reading = true;
        Next::Read(Reading(self))
    }

    /// The link mode the stick listens in, where it was set to.
    pub(crate) fn listening(&self) -> Option<u8> {
        self.kept.locked().listening
    }

    /// The stick set to listen in `link`, or to be set again where `None`.
    pub(crate) fn listens(&self, link: Option<u8>) {
        self.kept.locked().listening = link;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METER: [u8; 8] = [1; 8];
    const OTHER: [u8; 8] = [2; 8];

    #[test]
    fn a_telegram_is_kept_for_the_location_listening_for_its_meter() {
        let heard = Heard::default();
        heard.listen_for(METER);
        heard.keep(OTHER, vec![2]);
        heard.keep(METER, vec![1]);
        assert!(matches!(heard.next(METER, Duration::ZERO), Next::Heard(frame) if frame == [1]));
        let Next::Read(turn) = heard.next(OTHER, Duration::ZERO) else {
            panic!("nobody listens for the other meter: nothing kept, the port free");
        };
        assert!(
            matches!(heard.next(METER, Duration::ZERO), Next::Quiet),
            "one reads"
        );
        drop(turn);
        assert!(
            matches!(heard.next(METER, Duration::ZERO), Next::Read(_)),
            "let go"
        );
        for n in 0..=KEPT {
            heard.keep(METER, vec![u8::try_from(n).unwrap_or(0)]);
        }
        let Next::Heard(oldest) = heard.next(METER, Duration::ZERO) else {
            panic!("kept");
        };
        assert_eq!(oldest, [1], "the oldest let go first");
        heard.forget(METER);
        assert!(
            matches!(heard.next(METER, Duration::ZERO), Next::Read(_)),
            "forgotten"
        );
    }

    #[test]
    fn a_broadcast_location_takes_what_no_other_listens_for() {
        let heard = Heard::default();
        heard.listen_for(BROADCAST);
        heard.listen_for(METER);
        heard.keep(OTHER, vec![2]);
        heard.keep(METER, vec![1]);
        assert!(matches!(heard.next(BROADCAST, Duration::ZERO), Next::Heard(f) if f == [2]));
        assert!(matches!(heard.next(METER, Duration::ZERO), Next::Heard(f) if f == [1]));
    }

    /// A receive waiting while another reads the port is told the moment
    /// a telegram of its meter is kept.
    #[test]
    fn a_waiting_receive_is_told_what_the_reading_one_kept() {
        let heard = Heard::default();
        heard.listen_for(METER);
        let Next::Read(turn) = heard.next(OTHER, Duration::ZERO) else {
            panic!("the port free");
        };
        std::thread::scope(|scope| {
            let waiting = scope.spawn(|| match heard.next(METER, Duration::from_secs(10)) {
                Next::Heard(frame) => frame,
                _ => Vec::new(),
            });
            heard.keep(METER, vec![7]);
            assert_eq!(waiting.join().expect("joined"), [7]);
        });
        heard.listens(Some(3));
        assert_eq!(heard.listening(), Some(3));
        drop(turn);
    }
}
