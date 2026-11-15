# Third-party credits

| Component | Credit and licence |
| --- | --- |
| [chiaki-ng / libchiaki](https://github.com/streetpea/chiaki-ng) | StreetPea and contributors, based on [Chiaki](https://github.com/thestr4ng3r/chiaki) by Florian Märkl and contributors. See the submodule’s `COPYING` and `LICENSES` for AGPL-3.0 and the OpenSSL exception. |
| [Jerasure](https://github.com/ceph/jerasure) and [GF-Complete](https://github.com/ceph/gf-complete) | University of Tennessee and contributors. See the licences in the respective submodules. |
| [Nanopb](https://github.com/nanopb/nanopb) | Petteri Aimonen and contributors. See the licence in the submodule. |
| [Geist](https://github.com/vercel/geist-font) | The Geist Project Authors. SIL Open Font License 1.1. See [the included licence](assets/fonts/OFL.txt). |
| [FFmpeg](https://ffmpeg.org/) and [Opus](https://opus-codec.org/) | FFmpeg contributors and the Opus project contributors. Used for video and audio decoding. |
| [curl](https://curl.se/), [OpenSSL](https://www.openssl.org/), [json-c](https://github.com/json-c/json-c), [Libevent](https://libevent.org/), [MiniUPnP](https://miniupnp.tuxfamily.org/), and [zlib](https://zlib.net/) | Native components linked through libchiaki or vcpkg dependencies. |
| [egui / eframe](https://github.com/emilk/egui), [wgpu](https://github.com/gfx-rs/wgpu), [CPAL](https://github.com/RustAudio/cpal), [gilrs](https://gitlab.com/gilrs-project/gilrs), and [resvg](https://github.com/linebender/resvg) | Rust UI, graphics, audio, input, and SVG rendering. See `Cargo.lock` for the complete resolved crate list. |

Binary release archives contain the dependency licence files. The corresponding source download includes the PS Stream source, the Chiaki dependencies used by the build, the Windows patches, and the pinned build instructions. The FFmpeg source download contains the patched FFmpeg source used for the release DLLs.
