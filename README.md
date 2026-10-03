# xmip-core-transport-wireless-m-bus

Wireless M-Bus transport: EN 13757-4 over the air — the link layer frame of length, control, manufacturer, address and control information with a CRC per block, carrying the same records and the same meter as wired M-Bus; a loopback radio stands in for the receiver. A technology of [xmip-core-transport](https://github.com/IlleNilsson/xmip-core-transport).

## Acknowledgement

What a meter sends unasked, `SND_NR`, is at-most-once: the meter waits for no
reply, and the telegram is off the air as it is heard. What a receive reads
by `REQ_UD2` consumes nothing at the meter, which keeps holding it, so the
verdict has nothing to tell the meter, whichever it is: a receive cycle that
did not complete loses nothing, and the next read finds it again. Each Stream
arrives whole.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
