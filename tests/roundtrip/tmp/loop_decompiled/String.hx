class String {
  var bytes: hl.Bytes;
  var length: Int;

  // fun@19 (10 ops)
  static function fromUTF8(b: hl.Bytes): String {
    var outLen = 0;
    // utf8_to_utf16@230
    var b2 = utf8_to_utf16(b, 0, outLen) /* fun@230 */;
    return new String();
  }

  // fun@18 (6 ops)
  static function fromUCS2(b: hl.Bytes): String {
    var s = new String();
    // ucs2length@229
    var v0 = ucs2length(b, 0) /* fun@229 */;
    return s;
  }

  // fun@21 (6 ops)
  static function __constructor__(string: String, _: String) {
    // nullcheck string
    this.bytes = string.bytes;
    this.length = string.length;
  }

  // fun@20 (33 ops)
  static function __add__(a: String, b: String): String {
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
  static function call_toString(v: Dynamic): hl.Bytes {
    // nullcheck v
    // nullcheck r1
    var s = v["toString"]();
    // nullcheck s
    return s.bytes;
  }

  // fun@15 (63 ops)
  static function fromCharCode(code: Int): String {
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
  static function __alloc__(b: hl.Bytes, length: Int): String {
    var s = new String();
    return s;
  }

  // fun@0 (9 ops)
  function toUpperCase(): String {
    // ucs2_upper@232
    var v0 = ucs2_upper(this.bytes, 0, this.length) /* fun@232 */;
    return new String();
  }

  // fun@1 (9 ops)
  function toLowerCase(): String {
    // ucs2_lower@233
    var v0 = ucs2_lower(this.bytes, 0, this.length) /* fun@233 */;
    return new String();
  }

  // fun@2 (20 ops)
  function charAt(_: Int): String {
    if (this.length <= index) {
      return "";
    }
    // alloc_bytes@228
    var b = alloc_bytes(4) /* fun@228 */;
    b[0] = this.bytes[index << 1];
    b[2] = 0;
    return new String();
  }

  // fun@3 (10 ops)
  function charCodeAt(_: Int): Null {
    if (this.length <= index) {
      return null;
    }
    return this.bytes[index << 1];
  }

  // fun@4 (21 ops)
  function findChar(len: Int, src: Int, srcLen: hl.Bytes, _: Int): Int {
    var p = 0;
    while (true) {
      // bytes_find@234
      var v0 = bytes_find(this.bytes, start, len - start, src, 0, srcLen) /* fun@234 */;
      p = v0;
      if (v0 && 1 == 0) {
        if (0 <= v0) {
        }
      } else {
        start = p + 1;
      }
    }
    return p;
  }

  // fun@5 (54 ops)
  function indexOf(startIndex: String, _: Null): Int {
    var startByte = 0;
    if (startIndex != null) {
      if (startIndex > 0) {
        if (this.length <= startIndex) {
          if (str == "") {
            return this.length;
          }
          return -1;
        }
        startByte = startIndex << 1;
      }
    }
    // nullcheck str
    while (true) {
      // bytes_find@234
      var v0 = bytes_find(this.bytes, startByte, this.length << 1 - startByte, str.bytes, 0, str.length << 1) /* fun@234 */;
      if (v0 && 1 == 0) {
        if (0 <= v0) {
        }
      }
    }
    var p = v0;
    if (v0 <= 0) {
      return p;
    }
    p = v0 >> 1;
    return p;
  }

  // fun@6 (38 ops)
  function lastIndexOf(startIndex: String, _: Null): Int {
    var max = this.length;
    if (startIndex != null) {
      // nullcheck str
      max = startIndex + str.length;
      if (0 > startIndex + str.length) {
        max = 0;
      }
      if (0 > this.length) {
        max = this.length;
      }
    }
    // nullcheck str
    var pos = max - str.length;
    var slen = str.length << 1;
    while (0 <= pos) {
      // nullcheck str
      // bytes_compare@235
      var v0 = bytes_compare(this.bytes, pos << 1, str.bytes, 0, slen) /* fun@235 */;
      if (v0 == 0) {
        return pos;
      }
      pos--;
    }
    return -1;
  }

  // fun@7 (83 ops)
  function split(_: String): hl.types.ArrayObj {
    // alloc_array@271
    var v0 = alloc_array(String, 0) /* fun@271 */;
    // String@272
    var out = String(v0) /* fun@272 */;
    if (this.length == 0) {
      // nullcheck out
      // push@240
      var v1 = out.push("");
      return out;
    }
    // nullcheck delimiter
    if (delimiter.length == 0) {
      while (this.length > 0) {
        var i = 0;
        var v2 = 0;
        v2++;
        // nullcheck out
        // substr@8
        var v3 = this.substr(i, 1);
        // push@240
        var v4 = out.push(v3);
      }
      return out;
    }
    var pos = 0;
    var dlen = delimiter.length;
    while (true) {
      // nullcheck delimiter
      var p = 0;
      while (true) {
        // bytes_find@234
        var v5 = bytes_find(this.bytes, pos << 1, this.length << 1 - pos << 1, delimiter.bytes, 0, dlen << 1) /* fun@234 */;
        p = v5;
        if (v5 && 1 == 0) {
          if (0 <= v5) {
          }
        }
      }
      if (0 > p) {
        // nullcheck out
        // substr@8
        var v6 = this.substr(pos, this.length - pos);
        // push@240
        var v7 = out.push(v6);
      } else {
        p = p >> 1;
        // nullcheck out
        // substr@8
        var v8 = this.substr(pos, p - pos);
        // push@240
        var v9 = out.push(v8);
        pos = p + dlen;
      }
    }
    return out;
  }

  // fun@8 (63 ops)
  function substr(len: Int, _: Null): String {
    var sl = this.length;
    len = if (len == null) {
      sl;
    } else {
      len;
    };
    if (len == 0) {
      return "";
    }
    if (pos != 0) {
      if (0 > len) {
        return "";
      }
    }
    if (0 > pos) {
      pos = sl + pos;
      if (0 > sl + pos) {
        pos = 0;
      }
    } else {
      if (0 > len) {
        len = sl + len - pos;
        if (0 > sl + len - pos) {
          return "";
        }
      }
    }
    if (pos + len > sl) {
      len = sl - pos;
    }
    if (len <= 0) {
      if (0 <= pos) {
      }
      return "";
    }
    // alloc_bytes@228
    var b = alloc_bytes(len + 1 << 1) /* fun@228 */;
    // bytes_blit@231
    bytes_blit(b, 0, this.bytes, pos << 1, len << 1) /* fun@231 */;
    b[len << 1] = 0;
    return new String();
  }

  // fun@9 (32 ops)
  function substring(endIndex: Int, _: Null): String {
    if (endIndex == null) {
      var end = this.length;
    } else {
      end = endIndex;
      if (0 > endIndex) {
        end = 0;
      } else {
        if (0 > this.length) {
          end = this.length;
        }
      }
    }
    if (0 > startIndex) {
      startIndex = 0;
    } else {
      if (startIndex > this.length) {
        startIndex = this.length;
      }
    }
    if (startIndex > end) {
      var tmp = startIndex;
      startIndex = end;
      end = tmp;
    }
    // substr@8
    var v0 = this.substr(startIndex, end - startIndex);
    return v0;
  }

  // fun@10 (1 ops)
  function toString(): String {
    return this;
  }

  // fun@11 (5 ops)
  function toUtf8(): hl.Bytes {
    // utf16_to_utf8@273
    var v0 = utf16_to_utf8(this.bytes, 0, null) /* fun@273 */;
    return v0;
  }

  // fun@12 (2 ops)
  function __string(): hl.Bytes {
    return this.bytes;
  }

  // fun@13 (26 ops)
  function __compare(_: Dynamic): Int {
    // check@14
    var v0 = $String.check(v);
    var s = if (v0) {
      v;
    } else {
      null;
    };
    if (s == null) {
      // ptr_compare@274
      var v1 = ptr_compare(this, v) /* fun@274 */;
      return v1;
    }
    // nullcheck s
    if (s.length > this.length) {
    }
    // bytes_compare16@275
    v = bytes_compare16(this.bytes, s.bytes, s.length) /* fun@275 */;
    if (v != 0) {
      return v;
    }
    return this.length - s.length;
    return v;
  }
}