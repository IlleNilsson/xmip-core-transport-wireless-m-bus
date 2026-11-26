//! What a Location of this technology is configured with: its declaration,
//! which `Configured` reads, built with the crate's `declaration` feature
//! alone and the code behind the default (ADR-0064, amendments 2026-10-22
//! and 2026-11-26), for the library the surfaces load to carry (open
//! problem 48). The wait a Location takes unless told otherwise is declared
//! here, and the code takes it from here.

use std::time::Duration;

use xcore::settings::{Applies, Kind, Presence, Setting, Settings};

/// How long a device waits for a meter's answer unless told otherwise.
pub const TIMEOUT: Duration = Duration::from_secs(1);

/// A byte-sized part of a meter's identity.
const BYTE: Kind = Kind::Integer {
    minimum: 0,
    maximum: 255,
};

/// Every setting a Location takes beyond its address, the air: `loopback`,
/// a meter on an in-process radio, is the one the estate has; a receiver
/// joins when it exposes one. The meter is named by the four parts of its
/// link address.
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
        transport::configured::timeout(
            Applies::Both,
            Some(TIMEOUT),
            "How long a meter that does not answer is waited on.",
        ),
    ],
};

/// The schemes a Location's address is entered with to choose this technology.
pub const SCHEMES: &[&str] = &["wmbus"];
