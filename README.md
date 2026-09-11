# xmip-core-transport-wireless-m-bus

Wireless M-Bus transport: EN 13757-4 over the air — the link layer frame of length, control, manufacturer, address and control information with a CRC per block, carrying the same records and the same meter as wired M-Bus; a loopback radio stands in for the receiver. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
