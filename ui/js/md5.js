// MD5 of a byte array, as hex.
//
// The service identifies a save by `content_hash`, which is MD5 — chosen
// because that is what the library already stores for every ROM and save, not
// for any security property. Web Crypto offers SHA-1 and up and no MD5, so
// there is nothing to call and this is the alternative to changing a hash the
// desktop, the Flip and `inventory.db` all already agree on.
//
// Content addressing, not authentication. Nothing here defends against a
// deliberate collision and nothing needs to: both sides are the same house.
//
// Verified against RFC 1321's own test vectors, and cross-checked against the
// server's `md5_hex` on the same bytes — see md5.test.js.

const S = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
  5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
  4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
  6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];
// K[i] = floor(2^32 * abs(sin(i + 1))), computed rather than pasted so a typo
// in a 64-entry table is not a possibility.
const K = Array.from({ length: 64 }, (_, i) => Math.floor(Math.abs(Math.sin(i + 1)) * 2 ** 32));

const rotl = (x, c) => (x << c) | (x >>> (32 - c));

export function md5(bytes) {
  const msg = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const bitLen = msg.length * 8;
  // Pad to 56 mod 64, then eight bytes of little-endian bit length.
  const padded = new Uint8Array(((msg.length + 8) >> 6 << 6) + 64);
  padded.set(msg);
  padded[msg.length] = 0x80;
  const view = new DataView(padded.buffer);
  // Two 32-bit halves: a save can exceed 2^32 bits only in theory, but writing
  // the low word alone would be wrong rather than merely unlikely.
  view.setUint32(padded.length - 8, bitLen >>> 0, true);
  view.setUint32(padded.length - 4, Math.floor(bitLen / 2 ** 32), true);

  let [a0, b0, c0, d0] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
  const m = new Uint32Array(16);
  for (let off = 0; off < padded.length; off += 64) {
    for (let i = 0; i < 16; i++) m[i] = view.getUint32(off + i * 4, true);
    let [a, b, c, d] = [a0, b0, c0, d0];
    for (let i = 0; i < 64; i++) {
      let f, g;
      if (i < 16) { f = (b & c) | (~b & d); g = i; }
      else if (i < 32) { f = (d & b) | (~d & c); g = (5 * i + 1) % 16; }
      else if (i < 48) { f = b ^ c ^ d; g = (3 * i + 5) % 16; }
      else { f = c ^ (b | ~d); g = (7 * i) % 16; }
      const tmp = d;
      d = c;
      c = b;
      b = (b + rotl((a + f + K[i] + m[g]) | 0, S[i])) | 0;
      a = tmp;
    }
    a0 = (a0 + a) | 0; b0 = (b0 + b) | 0; c0 = (c0 + c) | 0; d0 = (d0 + d) | 0;
  }
  const out = new Uint8Array(16);
  new DataView(out.buffer).setUint32(0, a0 >>> 0, true);
  new DataView(out.buffer).setUint32(4, b0 >>> 0, true);
  new DataView(out.buffer).setUint32(8, c0 >>> 0, true);
  new DataView(out.buffer).setUint32(12, d0 >>> 0, true);
  return [...out].map((b) => b.toString(16).padStart(2, "0")).join("");
}
