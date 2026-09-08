// MD5, against the vectors in RFC 1321 itself.
//
// Hand-written because Web Crypto has no MD5 and the service identifies a save
// by one — a hash the desktop, the Flip and inventory.db already agree on, so
// the alternative was changing all of them. Hand-written code that computes a
// hash is exactly the kind of thing that is subtly wrong on one input and
// right on the rest, hence the vectors and the size sweep below.

import { test, describe } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { md5 } from "../js/md5.js";

const bytes = (s) => new TextEncoder().encode(s);

describe("md5", () => {
  /// RFC 1321, appendix A.5.
  test("the RFC's own test suite", () => {
    const vectors = [
      ["", "d41d8cd98f00b204e9800998ecf8427e"],
      ["a", "0cc175b9c0f1b6a831c399e269772661"],
      ["abc", "900150983cd24fb0d6963f7d28e17f72"],
      ["message digest", "f96b697d7cb7938d525a2f31aaf161d0"],
      ["abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"],
      ["ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
       "d174ab98d277d9f5a5611c2c9f419d9f"],
      ["1234567890".repeat(8), "57edf4a22be3c955ac49da2e2107b67a"],
    ];
    for (const [input, want] of vectors) {
      assert.equal(md5(bytes(input)), want, JSON.stringify(input.slice(0, 20)));
    }
  });

  /// The padding boundaries are where a hand-written MD5 goes wrong: exactly
  /// 55, 56, 64 and 120 bytes each take a different path through it.
  test("every length from 0 to 200 matches Node's own MD5", () => {
    for (let n = 0; n <= 200; n++) {
      const b = new Uint8Array(n);
      for (let i = 0; i < n; i++) b[i] = (i * 31 + 7) & 0xff;
      const want = createHash("md5").update(b).digest("hex");
      assert.equal(md5(b), want, `length ${n}`);
    }
  });

  /// A real save: 8 KB of SNES SRAM, and a 128 KB one for a bigger cartridge.
  test("save-sized inputs match", () => {
    for (const n of [8192, 32768, 131072]) {
      const b = new Uint8Array(n);
      for (let i = 0; i < n; i++) b[i] = (i * 7) & 0xff;
      assert.equal(md5(b), createHash("md5").update(b).digest("hex"), `${n} bytes`);
    }
  });

  test("high bytes are not treated as signed", () => {
    const b = new Uint8Array([0xff, 0x80, 0x00, 0x7f, 0xff]);
    assert.equal(md5(b), createHash("md5").update(b).digest("hex"));
  });
});
