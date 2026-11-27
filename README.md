# xmip-core-transport-wireless-m-bus

Wireless M-Bus transport: EN 13757-4 over the air — the link layer frame of length, control, manufacturer, address and control information with a CRC per block, carrying the same records and the same meter as wired M-Bus; a USB receiver stick on a serial port, or a loopback radio in process. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## The air

A Location's address is the air its meter is on. `loopback` is a meter on an
in-process radio (`LoopbackRadio`). Any other address is the serial port a
USB receiver stick is on, `/dev/ttyUSB0` or `COM3`, opened by the serial
technology as every serial port is; the transport speaks to the stick
through `Receiver`, a `Line` like the loopback radio, so everything above the
air is the same for both.

## The receiver stick

**The stick is IMST's iU891A-XL or iU893A-XL**, and its protocol is the host
controller interface of IMST's Wireless M-Bus Gateway firmware: IMST,
*Wireless M-Bus Gateway Firmware — Host Controller Interface Protocol*,
version 2.4, 2026-06-03, document 4000/40140/0178,
[wireless-solutions.de](https://wireless-solutions.de/data/assets/uploads/WM_Bus_Gateway_HCI_Protocol_Specification_V2_4.pdf).

Why this one, as found on 2026-11-27:

- **What is used.** [wmbusmeters](https://github.com/wmbusmeters/wmbusmeters),
  the widely used open-source reader, lists the receivers it drives: IMST's iM871A and iU891A, Amber's (now Würth Elektronik's)
  AMB8465 and AMB3665, the CUL family, Radiocraft's RC1180, and an RTL-SDR
  radio through rtl_wmbus or rtl_433. The RTL-SDR ones are a radio the host
  decodes itself, with no host protocol and no serial port to speak to.
- **The iM871A is discontinued.** It is the first stick wmbusmeters lists,
  and IMST's
  [end-of-life notice](https://wireless-solutions.de/data/assets/uploads/iM871A_iM871A_usb_EoL_Notification_2.pdf)
  names the iU891A-XL and the iU893A-XL its successors; IMST's own shop lists
  the iM871A-USB as discontinued. Its protocol, the older *WMBus HCI
  Specification*, answered 404 at the address IMST's shop gave it.
- **The successors' protocol is one public document, current and complete.**
  It is downloaded without registering; it gives the framing (SLIP, RFC
  1055), the check (CRC-16/IBM-SDLC, a catalogued CRC with its check value),
  every message, and the bytes of a ping, which this crate's tests hold. The
  format is IMST's WiMOD one, which its other products speak too.
- **The AMB8465's is two documents.** Würth's manual of the stick (the
  Metis-I USB radio stick, 2605056083001, formerly AMB8465-M) leaves its
  command set to the manual of the module inside it, which refused an
  automated download (403).

What the transport does with the stick:

- **At the first use of its port**, and again after a use failed, as a stick
  unplugged and plugged in again forgets it: the active configuration is read,
  its link mode set to the Location's `mode`, every heard telegram handed to
  the host and the address filter taken off, and it is written back, the
  LED, the calibration and the rest as they were. It is kept in RAM, not
  the stick's default configuration.
- **`mode` is the meter's**, `s`, `t` or `c`, `t` where it is left out: the
  stick listens in S, or in C and T at once, and sends to the meter in
  format A, in S's Manchester for S and T and at 50 kbit/s for C, what
  EN 13757-4 gives the other device in each mode's bidirectional form
  ([Radiocrafts' overview](https://radiocrafts.com/?p=8544)). **`power`** is what it sends
  at, -1 to +13 dBm, 10 where it is left out: the specification lists the
  range of one module, -1 to +13, and of the sticks none yet.
- **A frame goes out plain** in one send request (Send WM-Bus Message,
  `31h`), the stick putting on its length and CRCs, the port held until the
  stick's response, which comes within a second as the specification has
  it; a response that does not come is worth asking again, and a status
  that refuses is said in the specification's words. **A telegram comes in**
  as the stick's event (`20h`), CRCs checked and stripped by the stick, and
  the transport reads it as the air carries it. Every other event is let
  go.
- **One stick, every Location of a node on it.** The node's Locations naming
  the port share its one port, as serial's do, and what the stick heard:
  each Location takes its own meter's telegrams, the one whose receive reads
  the port keeps the others' for their Locations and tells them at once,
  and one receive reads the port at a time while the others wait, holding
  nothing. A meter's last 16 telegrams are kept for its Location; a
  telegram heard from it before a request to it is no answer and is let go;
  a meter no Location listens for is let go.

Not built: the stick's encryption and decryption (its device list and keys,
modes 5 and 7), sending under the stick's own address (`39h`), scan mode,
the stick's firmware update and its reset over USB. The specification's
table of message identifiers gives Send WM-Bus Message as the radio
modules' alone, where its own section names the iU891A-XL too; this crate
goes by the section.

**No stick was plugged in** (the owner, 2026-11-27, on transports that need
hardware: *let's do your best for now. I'm not going to buy all that
hardware*). `stick::Stick` stands in for one: a meter on its air, it answers
each message as the firmware does. This crate's tests drive a Location
through it on a port in process on every platform, and on Unix on a
pseudo-terminal pair; the root's service test drives a node's Send Location
through it on a pseudo-terminal pair, which Windows has none of.

## Acknowledgement

What a meter sends unasked, `SND_NR`, is at-most-once: the meter waits for no
reply, and the telegram is off the air as it is heard. What a receive reads
by `REQ_UD2` consumes nothing at the meter, which keeps holding it, so the
verdict has nothing to tell the meter, whichever it is: a receive cycle that
did not complete loses nothing, and the next read finds it again. Each Stream
arrives whole.

## The declaration

What a Location of this technology is configured with is two `const`s in its `declaration` module, `src/declaration.rs`, beside the code, which `Configured` reads (ADR-0064, amendment 2026-11-26): `declaration::SETTINGS`, `manufacturer`, `ident`, `version`, `medium`, `mode`, `power` and `timeout`, and `SCHEMES`, `wmbus`; `timeout`'s default, `declaration::TIMEOUT`, is declared there too, the code takes it from there, and the crate root names it as before. Built with its `declaration` feature alone (`--no-default-features --features declaration`), the crate is those and nothing more: its code and every crate the code is built on — the m-bus technology, whose records and meter it carries, and the serial technology, whose port the stick is on, among them — are behind the default feature. So the library the surfaces load carries it without the code, through the gathering crate xmip-core-declaration (open problem 48), and the language server and the desktop suggest and check a wireless M-Bus Location's settings though no node in their process loaded the technology. `test/Declaration.Test.ps1` in the estate holds that the declaration builds nothing the transport capability does not.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
