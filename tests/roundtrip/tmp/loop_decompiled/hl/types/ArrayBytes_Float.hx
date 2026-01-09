package hl.types;

class ArrayBytes_Float extends ArrayBase {
  var bytes: hl.Bytes;
  var size: Int;

  // fun@81 (5 ops)
  static function allocF64(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_Float {
    var a = new hl.types.ArrayBytes_Float();
    return a;
  }

  // fun@79 (5 ops)
  static function allocUI16(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_hl_UI16 {
    var a = new hl.types.ArrayBytes_hl_UI16();
    return a;
  }

  // fun@78 (5 ops)
  static function allocI32(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_Int {
    var a = new hl.types.ArrayBytes_Int();
    return a;
  }

  // fun@80 (5 ops)
  static function allocF32(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_hl_F32 {
    var a = new hl.types.ArrayBytes_hl_F32();
    return a;
  }

  // fun@82 (32 ops)
  function concat(_: hl.types.ArrayBytes_Float): hl.types.ArrayBytes_Float {
    var ac = new hl.types.ArrayBytes_Float();
    ac = new hl.types.ArrayBytes_Float();
    // String@351
    // nullcheck a
    ac.size = this.length + a.length;
    ac.length = this.length + a.length;
    // alloc_bytes@228
    var v0 = alloc_bytes(ac.length << 3) /* fun@228 */;
    ac.bytes = v0;
    var offset = this.length << 3;
    // bytes_blit@231
    bytes_blit(ac.bytes, 0, this.bytes, 0, offset) /* fun@231 */;
    // bytes_blit@231
    bytes_blit(ac.bytes, offset, a.bytes, 0, a.length << 3) /* fun@231 */;
    return ac;
  }

  // fun@83 (23 ops)
  function join(_: String): String {
    var s = new StringBuf();
    s = new StringBuf();
    // __constructor__@276
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      if (i > 0) {
        // nullcheck s
        // add@277
        s.add(sep);
      }
      // nullcheck s
      // add@277
      s.add(this.bytes[i << 3]);
    }
    // nullcheck s
    // toString@279
    var v1 = s.toString();
    return v1;
  }

  // fun@84 (15 ops)
  function pop(): Null {
    if (this.length == 0) {
      return null;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    return this.bytes[this.length << 3];
  }

  // fun@85 (14 ops)
  function push(_: Float): Int {
    var len = this.length;
    if (this.size == len) {
      // __expand@115
      this.__expand(len);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    this.bytes[len << 3] = x;
    return this.length;
  }

  // fun@86 (30 ops)
  function reverse() {
    while (this.length >> 1 > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var k = this.length - 1 - i;
      var tmp = this.bytes[i << 3];
      this.bytes[i << 3] = this.bytes[k << 3];
      this.bytes[k << 3] = tmp;
    }
  }

  // fun@87 (27 ops)
  function shift(): Null {
    if (this.length == 0) {
      return null;
    }
    var v = this.bytes[0 << 3];
    var v0 = this.length;
    v0--;
    this.length = v0;
    // bytes_blit@231
    bytes_blit(this.bytes, 0, this.bytes, 1 << 3, this.length << 3) /* fun@231 */;
    return v;
  }

  // fun@88 (30 ops)
  function blit(src: Int, srcpos: hl.types.ArrayAccess, len: Int, _: Int) {
    src = src;
    if (srcpos + len > src.length) {
      if (0 <= pos) {
        if (0 <= srcpos) {
          if (0 <= len) {
            if (pos + len <= this.length) {
              // nullcheck src
            }
          }
        }
      }
      // thrown@227
      var v0 = haxe.io.Error.Blocked;
      throw v0;
    }
    // bytes_blit@231
    bytes_blit(this.bytes, pos << 3, src.bytes, srcpos << 3, len << 3) /* fun@231 */;
  }

  // fun@89 (40 ops)
  function slice(end: Int, _: Null): hl.types.ArrayBase {
    if (0 > pos) {
      pos = this.length + pos;
      if (0 > this.length + pos) {
        pos = 0;
      }
    }
    if (end == null) {
      var pend = this.length;
    } else {
      pend = end;
      if (0 > pend) {
        pend = pend + this.length;
      }
      if (pend > this.length) {
        pend = this.length;
      }
    }
    var len = pend - pos;
    if (0 > len) {
      // String@351
      return new hl.types.ArrayBytes_Float();
    }
    var a = new hl.types.ArrayBytes_Float();
    a = new hl.types.ArrayBytes_Float();
    // String@351
    a.size = len;
    a.length = len;
    // sub@287
    var v0 = sub(this.bytes, pos << 3, len << 3) /* fun@287 */;
    a.bytes = v0;
    return a;
  }

  // fun@90 (17 ops)
  function sort(_: Function) {
    if (Float == Int) {
      if (f == null) {
      } else {
        // closure : String@352
      }
      // bsort_i32@285
      bsort_i32(this.bytes, 0, this.length, f.String) /* fun@285 */;
    } else {
      // bsort_f64@286
      bsort_f64(this.bytes, 0, this.length, f) /* fun@286 */;
    }
  }

  // fun@91 (68 ops)
  function splice(len: Int, _: Int): hl.types.ArrayBase {
    if (0 > len) {
      // String@351
      return new hl.types.ArrayBytes_Float();
    }
    if (0 > pos) {
      pos = this.length + pos;
      if (0 > this.length + pos) {
        pos = 0;
      }
    }
    if (pos > this.length) {
      pos = 0;
      len = 0;
    } else {
      if (pos + len > this.length) {
        len = this.length - pos;
        if (0 > this.length - pos) {
          len = 0;
        }
      }
    }
    if (len == 0) {
      // String@351
      return new hl.types.ArrayBytes_Float();
    }
    var ret = new hl.types.ArrayBytes_Float();
    ret = new hl.types.ArrayBytes_Float();
    // String@351
    // sub@287
    var v0 = sub(this.bytes, pos << 3, len << 3) /* fun@287 */;
    ret.bytes = v0;
    ret.length = len;
    ret.size = len;
    var end = pos + len;
    // bytes_blit@231
    bytes_blit(this.bytes, pos << 3, this.bytes, end << 3, this.length - end << 3) /* fun@231 */;
    this.length = this.length - len;
    return ret;
  }

  // fun@92 (28 ops)
  function toString(): String {
    var b = new StringBuf();
    b = new StringBuf();
    // __constructor__@276
    // addChar@278
    b.addChar(91);
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      if (i > 0) {
        // nullcheck b
        // addChar@278
        b.addChar(44);
      }
      // nullcheck b
      // add@277
      b.add(this.bytes[i << 3]);
    }
    // nullcheck b
    // addChar@278
    b.addChar(93);
    // toString@279
    var v1 = b.toString();
    return v1;
  }

  // fun@93 (29 ops)
  function unshift(_: Float) {
    if (this.length == this.size) {
      // __expand@115
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // bytes_blit@231
    bytes_blit(this.bytes, 1 << 3, this.bytes, 0, this.length - 1 << 3) /* fun@231 */;
    this.bytes[0 << 3] = x;
  }

  // fun@94 (46 ops)
  function insert(x: Int, _: Float) {
    if (0 > pos) {
      pos = this.length + pos;
      if (0 > this.length + pos) {
        pos = 0;
      }
    } else {
      if (pos > this.length) {
        pos = this.length;
      }
    }
    if (this.length == this.size) {
      // __expand@115
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // bytes_blit@231
    bytes_blit(this.bytes, pos + 1 << 3, this.bytes, pos << 3, this.length - pos - 1 << 3) /* fun@231 */;
    this.bytes[pos << 3] = x;
  }

  // fun@95 (8 ops)
  function contains(_: Float): Bool {
    // indexOf@97
    var v0 = this.indexOf(x, null);
    if (v0 == -1) {
    }
    return true;
  }

  // fun@96 (27 ops)
  function remove(_: Float): Bool {
    // indexOf@97
    var idx = this.indexOf(x, null);
    if (0 > idx) {
      return false;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    // bytes_blit@231
    bytes_blit(this.bytes, idx << 3, this.bytes, idx + 1 << 3, this.length - idx << 3) /* fun@231 */;
    return true;
  }

  // fun@97 (28 ops)
  function indexOf(fromIndex: Float, _: Null): Int {
    var idx = if (fromIndex == null) {
      0;
    } else {
      fromIndex;
    };
    if (0 > idx) {
      idx = idx + this.length;
      if (0 > idx + this.length) {
        idx = 0;
      }
    }
    while (this.length > idx) {
      var i = idx;
      idx++;
      if (this.bytes[i << 3] == x) {
        return i;
      }
    }
    return -1;
  }

  // fun@98 (28 ops)
  function lastIndexOf(fromIndex: Float, _: Null): Int {
    var len = this.length;
    var i = if (fromIndex != null) {
      fromIndex;
    } else {
      len - 1;
    };
    if (len <= i) {
      i = len - 1;
    } else {
      if (0 > i) {
        i = i + len;
      }
    }
    while (0 <= i) {
      if (this.bytes[i << 3] == x) {
        return i;
      }
      i--;
    }
    return -1;
  }

  // fun@99 (21 ops)
  function copy(): hl.types.ArrayBytes_Float {
    var a = new hl.types.ArrayBytes_Float();
    a = new hl.types.ArrayBytes_Float();
    // String@351
    a.size = this.length;
    a.length = this.length;
    // alloc_bytes@228
    var v0 = alloc_bytes(this.length << 3) /* fun@228 */;
    a.bytes = v0;
    // bytes_blit@231
    bytes_blit(a.bytes, 0, this.bytes, 0, this.length << 3) /* fun@231 */;
    return a;
  }

  // fun@100 (3 ops)
  function iterator(): haxe.iterators.ArrayIterator {
    // __constructor__@355
    return new hl.types.BytesIterator_Float(this);
  }

  // fun@101 (3 ops)
  function keyValueIterator(): haxe.iterators.ArrayKeyValueIterator {
    // __constructor__@358
    return new hl.types.BytesKeyValueIterator_Float(this);
  }

  // fun@102 (29 ops)
  function map(_: Function): hl.types.ArrayDyn {
    var a = new hl.types.ArrayObj();
    a = new hl.types.ArrayObj();
    // String@300
    if (this.length > 0) {
      // __expand@260
      a.__expand(this.length - 1);
    }
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      // nullcheck a
      // nullcheck f
      a.array[i] = f(this.bytes[i << 3]);
    }
    // alloc@350
    var v1 = alloc(a, true) /* fun@350 */;
    return v1;
  }

  // fun@103 (19 ops)
  function filter(_: Function): hl.types.ArrayBytes_Float {
    var a = new hl.types.ArrayBytes_Float();
    a = new hl.types.ArrayBytes_Float();
    // String@351
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var v = this.bytes[i << 3];
      // nullcheck f
      if (f(v)) {
        // nullcheck a
        // push@85
        var v1 = a.push(v);
      }
    }
    return a;
  }

  // fun@104 (21 ops)
  function resize(_: Int) {
    if (len > this.length) {
      // __expand@115
      this.__expand(len - 1);
    } else {
      if (this.length <= len) {
        return;
      }
      // bytes_fill@284
      bytes_fill(this.bytes, len << 3, this.length - len << 3, 0) /* fun@284 */;
      this.length = len;
    }
  }

  // fun@105 (12 ops)
  function getDyn(_: Int): Dynamic {
    if (this.length <= pos) {
      return 0;
    }
    return this.bytes[pos << 3];
  }

  // fun@106 (9 ops)
  function setDyn(v: Int, _: Dynamic) {
    if (this.length <= pos) {
      // __expand@115
      this.__expand(pos);
    }
    this.bytes[pos << 3] = v;
  }

  // fun@107 (3 ops)
  function pushDyn(_: Dynamic): Int {
    // push@85
    var v0 = this.push(v);
    return v0;
  }

  // fun@108 (2 ops)
  function popDyn(): Dynamic {
    // pop@84
    var v0 = this.pop();
    return v0;
  }

  // fun@109 (2 ops)
  function shiftDyn(): Dynamic {
    // shift@87
    var v0 = this.shift();
    return v0;
  }

  // fun@110 (3 ops)
  function unshiftDyn(_: Dynamic) {
    // unshift@93
    this.unshift(v);
  }

  // fun@111 (3 ops)
  function insertDyn(v: Int, _: Dynamic) {
    // insert@94
    this.insert(pos, v);
  }

  // fun@112 (3 ops)
  function containsDyn(_: Dynamic): Bool {
    // contains@95
    var v0 = this.contains(v);
    return v0;
  }

  // fun@113 (3 ops)
  function removeDyn(_: Dynamic): Bool {
    // remove@96
    var v0 = this.remove(v);
    return v0;
  }

  // fun@114 (6 ops)
  function sortDyn(_: Function) {
    if (f == null) {
    } else {
      // closure : String@359
    }
    // sort@90
    this.sort(f.String);
  }

  // fun@115 (43 ops)
  function __expand(_: Int) {
    if (0 > index) {
      // itos@221
      var v0 = index;
      // __alloc__@16
      var v1 = v0;
      // __add__@20
      var v2 = "Invalid array index " + v1;
      // thrown@227
      var v3 = v2;
      throw v3;
    }
    var newlen = index + 1;
    if (newlen > this.size) {
      var next = this.size * 3 >> 1;
      if (newlen > next) {
        next = newlen;
      }
      // alloc_bytes@228
      var bytes2 = alloc_bytes(next << 3) /* fun@228 */;
      var bsize = this.length << 3;
      // bytes_blit@231
      bytes_blit(bytes2, 0, this.bytes, 0, bsize) /* fun@231 */;
      // bytes_fill@284
      bytes_fill(bytes2, bsize, next << 3 - bsize, 0) /* fun@284 */;
      this.bytes = bytes2;
      this.size = next;
    }
    this.length = newlen;
  }
}