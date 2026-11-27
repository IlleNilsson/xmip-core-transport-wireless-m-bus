//! This technology's declaration: the transport guide's "The declaration",
//! `module/core/capability/transport/doc/adding-a-transport.md`. The wait a
//! Location takes unless told otherwise is declared here, and the code takes
//! it from here.

use std::time::Duration;

use xcore::settings::{Applies, Fixed, Kind, Presence, Setting, Settings};

/// How long a device waits for a meter's answer unless told otherwise.
pub const TIMEOUT: Duration = Duration::from_secs(1);

/// The transmit power a receiver stick sends at unless its Location says
/// another, in dBm: well inside the -1 to +13 dBm IMST's specification
/// gives the one module it lists.
const POWER: i64 = 10;

/// A byte-sized part of a meter's identity.
const BYTE: Kind = Kind::Integer {
    minimum: 0,
    maximum: 255,
};

/// Every setting a Location takes beyond its address, the air: the serial
/// port a receiver stick is on, `/dev/ttyUSB0` or `COM3`, or `loopback`, a
/// meter on an in-process radio. The meter is named by the four parts of
/// its link address; on a stick, its mode is what the stick listens and
/// answers in, at its power.
pub const SETTINGS: &Settings = &Settings {
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
            name: "mode",
            kind: Kind::Choice {
                choices: &["s", "t", "c"],
            },
            presence: Presence::Default(Fixed::Text("t")),
            meaning: "The mode of EN 13757-4 the meter speaks, which a receiver stick listens \
                      in and answers it in: s, t or c.",
            applies: Applies::Both,
        },
        Setting {
            name: "power",
            kind: Kind::Integer {
                minimum: -1,
                maximum: 13,
            },
            presence: Presence::Default(Fixed::Integer(POWER)),
            meaning: "The power a receiver stick sends to the meter at, in dBm.",
            applies: Applies::Both,
        },
        transport::configured::timeout(
            Applies::Both,
            Some(TIMEOUT),
            "How long a meter that does not answer is waited on.",
        ),
    ],
};

/// This technology's schemes, as [`transport::Configured::SCHEMES`] defines
/// them.
pub const SCHEMES: &[&str] = &["wmbus"];
