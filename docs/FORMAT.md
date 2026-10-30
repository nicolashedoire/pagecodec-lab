# Experimental page format

`codec.c` is a lossless representation codec for caller-owned byte buffers. It
does not change the operating system, inspect other processes, or replace the
macOS memory compressor. Its adaptive choices use known ideas: repeated values,
base-and-delta packing, and sparse exceptions.

The format is experimental and has no embedded version number. Do not treat it
as a stable archival or interchange format. This document describes the V2
implementation committed in this repository.

## API contract

```c
size_t rc_bound(size_t n);
size_t rc_encode(uint8_t *dst, size_t cap, const uint8_t *src, size_t n);
bool rc_decode(uint8_t *dst, size_t n, const uint8_t *src, size_t m);
```

- `n` is the original byte length; the caller must store it separately.
- `m` is the encoded byte length, returned by `rc_encode`.
- `rc_bound(n)` returns `n + 1`, or zero if that bound would overflow.
- Encoding returns zero on failure. The destination may already contain partial
  candidate output after failure; only use the returned byte count on success.
- Decoding returns false when supported structural checks reject the frame.
- Source and destination buffers must not overlap. The caller supplies valid
  buffers of the indicated sizes. The implementation performs no allocations.
- The frame contains neither a checksum nor authentication. A structurally valid
  modified frame can decode to modified data. These checks are not an assurance
  against every malformed input or an independent security audit.

All multibyte frame fields and words are little endian. The first byte is a mode
tag; the original length is not repeated in the frame.

## Modes

| Tag | Name | Payload after tag | Frame size |
|---:|---|---|---:|
| 0 | RAW | The original `n` bytes | `1 + n` |
| 1 | BYTE_UNIFORM | One byte to repeat `n` times | 2 |
| 2 | REPEAT64 | An eight-byte pattern, repeated and truncated to `n` bytes | 9 |
| 3 | BYTE_DELTA4 | One-byte base followed by low-nibble-first four-bit residuals | `2 + ceil(n / 2)` |
| 4 | LANES32 | Four width bytes, four uint32 bases, packed lane streams, raw tail | See below |
| 5 | SPARSE32 | A uint32 base, uint16 count, sorted exception records, raw tail | `7 + 6 × count + n % 4` |

The encoder can return a RAW frame for an empty input. The surrounding benchmark
container also has its own raw-page marker and discards any encoded frame that
does not save space, including the frame's header. That container marker is
separate from the six codec tags.

The decoder requires `n > 0` for BYTE_UNIFORM and BYTE_DELTA4, and `n >= 8`
for REPEAT64. The encoder selects BYTE_UNIFORM only from two bytes and REPEAT64
only from 16 bytes; decoding also permits the shorter valid frames above.

### LANES32

Split the first `floor(n / 4)` words into four interleaved lanes: lane `l` holds
word indices `l`, `l + 4`, `l + 8`, and so on. The header is 21 bytes:

1. One-byte mode tag.
2. Four one-byte widths in lane order.
3. Four little-endian uint32 bases in lane order.

Widths are one of `0, 4, 8, 12, 16, 20, 24, 28, 32`. Subtraction and reconstruction
use uint32 arithmetic modulo `2^32`, allowing a compact signed interval that
crosses zero without a separate signedness flag. Width zero means every word in
that lane equals its base.

Residual streams follow in lane order. Byte-aligned widths use little-endian
residuals. Widths ending in four bits pack each pair of residuals consecutively,
least significant bits first; an odd final residual occupies `ceil(width / 8)`
bytes. Each lane is byte-aligned independently. Finally, copy the original
`n % 4` tail bytes verbatim. The decoder checks supported widths and the exact
frame size implied by the caller's original length.

### SPARSE32

After the mode tag and four-byte base, a two-byte count specifies the number of
exceptions. Each six-byte exception consists of a uint16 word index and the
original uint32 word. Indices must be strictly increasing and smaller than
`floor(n / 4)`. The decoder validates the list before filling the output with
the base and applying exceptions. The original `n % 4` bytes follow the list.

This mode supports from 2 through 65,536 complete words. The encoder estimates a
background from the first three words, probes at most 32 words, and requires at
least 75% of those sampled words to equal the estimated background before
scanning the full page. Missing a suitable background can reduce compression,
but does not permit a lossy output.

When the sparse representation occupies at most `n / 16`, the encoder can skip
the lane-extrema pass. This fixed policy favors encoding cost and does not
guarantee the smallest possible representation among all six modes.

## Verification scope

The portable suite covers round trips, unaligned buffers, partial words,
capacity limits, mode boundaries, truncated frames and selected malformed
sparse-index lists. AddressSanitizer and UndefinedBehaviorSanitizer are enabled
by `make sanitize`. The macOS comparator smoke tests additionally check byte
checksums and page-table accounting. CI runs these correctness checks only;
shared-runner timings are not used as performance evidence.
