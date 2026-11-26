# xmip-core-transport-wireless-m-bus

Wireless M-Bus transport: EN 13757-4 over the air — the link layer frame of length, control, manufacturer, address and control information with a CRC per block, carrying the same records and the same meter as wired M-Bus; a loopback radio stands in for the receiver. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## Acknowledgement

What a meter sends unasked, `SND_NR`, is at-most-once: the meter waits for no
reply, and the telegram is off the air as it is heard. What a receive reads
by `REQ_UD2` consumes nothing at the meter, which keeps holding it, so the
verdict has nothing to tell the meter, whichever it is: a receive cycle that
did not complete loses nothing, and the next read finds it again. Each Stream
arrives whole.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `manufacturer`, `ident`, `version`, `medium` and `timeout`, and `SCHEMES`, `wmbus`; `timeout`'s default, `declaration::TIMEOUT`, is declared there too, the code takes it from there, and the crate root names it as before. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on — the m-bus technology, whose records and meter it carries, among them — are behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check a wireless M-Bus Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
