class $String extends Class {

  // fun@19 (10 ops)
  dynamic function fromUTF8(): String {
    var outLen = 0;
    // utf8_to_utf16@230
    var b2 = utf8_to_utf16(b, 0, outLen) /* fun@230 */;
    return new String();
  }

  // fun@18 (6 ops)
  dynamic function fromUCS2(): String {
    var s = new String();
    // ucs2length@229
    var v0 = ucs2length(b, 0) /* fun@229 */;
    return s;
  }

  // fun@21 (6 ops)
  dynamic function __constructor__(_: String) {
    // nullcheck string
    this.bytes = string.bytes;
    this.length = string.length;
  }

  // fun@20 (33 ops)
  dynamic function __add__(b: String): String {
    if (a == null) {
      a = "null";
    }
    if (b == null) {
      b = "null";
    }
    // nullcheck a
    var asize = a.length << 1;
    // nullcheck b
    var bsize = b.length << 1;
    var tot = asize + bsize;
    // alloc_bytes@228
    var bytes = alloc_bytes(tot + 2) /* fun@228 */;
    // bytes_blit@231
    bytes_blit(bytes, 0, a.bytes, 0, asize) /* fun@231 */;
    // bytes_blit@231
    bytes_blit(bytes, asize, b.bytes, 0, bsize) /* fun@231 */;
    bytes[tot] = 0;
    return new String();
  }

  // fun@17 (8 ops)
  dynamic function call_toString(): hl.Bytes {
    // nullcheck v
    // nullcheck r1
    var s = v["toString"]();
    // nullcheck s
    return s.bytes;
  }

  // fun@15 (63 ops)
  dynamic function fromCharCode(): String {
    if (0 <= code) {
      if (65536 > code) {
        if (55296 <= code) {
          if (code <= 57343) {
            // itos@221
            var v0 = code;
            // __alloc__@16
            var v1 = v0;
            // __add__@20
            var v2 = "Invalid unicode char " + v1;
            // thrown@227
            var v3 = v2;
            throw v3;
          }
        }
        // alloc_bytes@228
        var b = alloc_bytes(4) /* fun@228 */;
        b[0] = code;
        b[2] = 0;
        return new String();
      }
    }
    if (1114112 > code) {
      // alloc_bytes@228
      b = alloc_bytes(6) /* fun@228 */;
      code = code - 65536;
      b[0] = code >> 10 + 55296;
      b[2] = code && 1023 + 56320;
      b[4] = 0;
      return new String();
    }
    // itos@221
    var v4 = code;
    // __alloc__@16
    var v5 = v4;
    // thrown@227
    throw new String();
  }

  // fun@16 (4 ops)
  dynamic function __alloc__(length: Int): String {
    var s = new String();
    return s;
  }
}