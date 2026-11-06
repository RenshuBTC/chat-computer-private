# picture

Chat Computer's own picture decoders (JPEG, PNG, GIF, WebP, and SVG drawn), its JPEG and PNG writers, and the
resampling Qwen's processor does. See the crate's `src/lib.rs`, and `os/README.md`.

## WebP's tables

The tables VP8's decoding needs (`src/webp_tables.rs`: the default coefficient probabilities and the probabilities
of their updates, a key frame's 4 × 4 prediction modes' probabilities, the dequantization factors, the coefficients'
zigzag order and bands, the DCT token categories' extra bits' probabilities) are RFC 6386's, and VP8L's (its 120
short distance codes in 2D, and the order of its code length code lengths) RFC 9649's, as libwebp has them, at commit
`d19a90a30e9b2cc5aa66e5fe71d2f2d9d61588c4` (`src/dec/tree_dec.c`, `src/dec/quant_dec.c`, `src/dec/vp8_dec.c`,
`src/dec/vp8l_dec.c`), numbers only, written out as Rust. They're in libwebp's order of the 4 × 4 modes (DC, TM, VE,
HE, RD, VR, LD, VL, HD, HU), which the decoder uses. The decoders themselves are written here, from the two RFCs,
and checked against libwebp's decoding (through Pillow), which they match exactly (`tests/cases.py`).

Those tables are under libwebp's licence:

```
Copyright (c) 2010, Google Inc. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of Google nor the names of its contributors may
    be used to endorse or promote products derived from this software
    without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```
