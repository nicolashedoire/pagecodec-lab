#ifndef RAM_CHALLENGE_CODEC_H
#define RAM_CHALLENGE_CODEC_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Independent experimental page representation. No allocations or external codec.
 * Encoded bytes include the mode/header. Caller stores the original size n.
 * src/dst must not overlap. Bound is n+1; zero denotes size overflow.
 * Decoder validates frame structure, not authenticity or checksums.
 */
size_t rc_bound(size_t n);
size_t rc_encode(uint8_t *dst, size_t cap, const uint8_t *src, size_t n);
bool rc_decode(uint8_t *dst, size_t n, const uint8_t *src, size_t m);
#endif
