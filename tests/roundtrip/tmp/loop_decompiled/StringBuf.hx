class StringBuf {
  var b: hl.Bytes;
  var size: Int;
  var pos: Int;

  // fun@276 (8 ops)
  static function __constructor__(_: StringBuf) {
    this.pos = 0;
    this.size = 8;
    // alloc_bytes@228
    var v0 = alloc_bytes(this.size) /* fun@228 */;
    this.b = v0;
  }

  // fun@277 (75 ops)
  function add(_: Dynamic) {
    var slen = 0;
    // check@14
    var v0 = $String.check(x);
    var str = if (v0) {
      x;
    } else {
      null;
    };
    if (str != null) {
      // nullcheck str
      if (this.pos + str.length << 1 > this.size) {
        if (this.pos + str.length << 1 > this.size * 3 >> 1) {
        }
        // alloc_bytes@228
        var v1 = alloc_bytes(this.pos + str.length << 1) /* fun@228 */;
        // bytes_blit@231
        bytes_blit(v1, 0, this.b, 0, this.pos) /* fun@231 */;
        this.b = v1;
        this.size = this.pos + str.length << 1;
      }
      // bytes_blit@231
      bytes_blit(this.b, this.pos, str.bytes, 0, str.length << 1) /* fun@231 */;
      this.pos = this.pos + str.length << 1;
      return;
    }
    // value_to_string@225
    var sbytes = value_to_string(x, slen) /* fun@225 */;
    if (this.pos + slen << 1 > this.size) {
      if (this.pos + slen << 1 > this.size * 3 >> 1) {
      }
      // alloc_bytes@228
      var v2 = alloc_bytes(this.pos + slen << 1) /* fun@228 */;
      // bytes_blit@231
      bytes_blit(v2, 0, this.b, 0, this.pos) /* fun@231 */;
      this.b = v2;
      this.size = this.pos + slen << 1;
    }
    // bytes_blit@231
    bytes_blit(this.b, this.pos, sbytes, 0, slen << 1) /* fun@231 */;
    this.pos = this.pos + slen << 1;
  }

  // fun@278 (103 ops)
  function addChar(_: Int) {
    if (65536 > c) {
      if (55296 <= c) {
        if (c <= 57343) {
          // itos@221
          var v0 = c;
          // __alloc__@16
          var v1 = v0;
          // __add__@20
          var v2 = "Invalid unicode char " + v1;
          // thrown@227
          var v3 = v2;
          throw v3;
        }
      }
      if (this.pos + 2 > this.size) {
        if (0 > this.size * 3 >> 1) {
        }
        // alloc_bytes@228
        var v4 = alloc_bytes(0) /* fun@228 */;
        // bytes_blit@231
        bytes_blit(v4, 0, this.b, 0, this.pos) /* fun@231 */;
        this.b = v4;
        this.size = 0;
      }
      this.b[this.pos] = c;
      this.pos = this.pos + 2;
    } else {
      if (0 <= c) {
      }
      if (1114112 > c) {
        if (this.pos + 4 > this.size) {
          if (0 > this.size * 3 >> 1) {
          }
          // alloc_bytes@228
          var v5 = alloc_bytes(0) /* fun@228 */;
          // bytes_blit@231
          bytes_blit(v5, 0, this.b, 0, this.pos) /* fun@231 */;
          this.b = v5;
          this.size = 0;
        }
        c = c - 65536;
        this.b[this.pos] = c >> 10 + 55296;
        this.b[this.pos + 2] = c && 1023 + 56320;
        this.pos = this.pos + 4;
      } else {
        // itos@221
        var v6 = c;
        // __alloc__@16
        var v7 = v6;
        // __add__@20
        var v8 = "Invalid unicode char " + v7;
        // thrown@227
        var v9 = v8;
        throw v9;
      }
    }
  }

  // fun@279 (34 ops)
  function toString(): String {
    if (this.pos + 2 > this.size) {
      if (0 > this.size * 3 >> 1) {
      }
      // alloc_bytes@228
      var v0 = alloc_bytes(0) /* fun@228 */;
      // bytes_blit@231
      bytes_blit(v0, 0, this.b, 0, this.pos) /* fun@231 */;
      this.b = v0;
      this.size = 0;
    }
    this.b[this.pos] = 0;
    return new String();
  }

  // fun@280 (4 ops)
  function __string(): hl.Bytes {
    // toString@279
    var v0 = this.toString();
    // nullcheck v0
    return v0.bytes;
  }
}