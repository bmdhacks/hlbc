package hl.types;

class ArrayBytes_Int extends ArrayBase {
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

  // fun@185 (32 ops)
  function concat(_: hl.types.ArrayBytes_Int): hl.types.ArrayBytes_Int {
    var ac = new hl.types.ArrayBytes_Int();
    ac = new hl.types.ArrayBytes_Int();
    // String@360
    // nullcheck a
    ac.size = this.length + a.length;
    ac.length = this.length + a.length;
    // alloc_bytes@228
    var v0 = alloc_bytes(ac.length << 2) /* fun@228 */;
    ac.bytes = v0;
    var offset = this.length << 2;
    // bytes_blit@231
    bytes_blit(ac.bytes, 0, this.bytes, 0, offset) /* fun@231 */;
    // bytes_blit@231
    bytes_blit(ac.bytes, offset, a.bytes, 0, a.length << 2) /* fun@231 */;
    return ac;
  }

  // fun@186 (23 ops)
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
      s.add(this.bytes[i << 2]);
    }
    // nullcheck s
    // toString@279
    var v1 = s.toString();
    return v1;
  }

  // fun@187 (15 ops)
  function pop(): Null {
    if (this.length == 0) {
      return null;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    return this.bytes[this.length << 2];
  }

  // fun@188 (14 ops)
  function push(_: Int): Int {
    var len = this.length;
    if (this.size == len) {
      // __expand@218
      this.__expand(len);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    this.bytes[len << 2] = x;
    return this.length;
  }

  // fun@189 (30 ops)
  function reverse() {
    while (this.length >> 1 > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var k = this.length - 1 - i;
      var tmp = this.bytes[i << 2];
      this.bytes[i << 2] = this.bytes[k << 2];
      this.bytes[k << 2] = tmp;
    }
  }

  // fun@190 (27 ops)
  function shift(): Null {
    if (this.length == 0) {
      return null;
    }
    var v = this.bytes[0 << 2];
    var v0 = this.length;
    v0--;
    this.length = v0;
    // bytes_blit@231
    bytes_blit(this.bytes, 0, this.bytes, 1 << 2, this.length << 2) /* fun@231 */;
    return v;
  }

  // fun@191 (30 ops)
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
    bytes_blit(this.bytes, pos << 2, src.bytes, srcpos << 2, len << 2) /* fun@231 */;
  }

  // fun@192 (40 ops)
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
      // String@360
      return new hl.types.ArrayBytes_Int();
    }
    var a = new hl.types.ArrayBytes_Int();
    a = new hl.types.ArrayBytes_Int();
    // String@360
    a.size = len;
    a.length = len;
    // sub@287
    var v0 = sub(this.bytes, pos << 2, len << 2) /* fun@287 */;
    a.bytes = v0;
    return a;
  }

  // fun@193 (17 ops)
  function sort(_: Function) {
    if (Int == Int) {
      // bsort_i32@285
      bsort_i32(this.bytes, 0, this.length, f) /* fun@285 */;
    } else {
      if (f == null) {
      } else {
        // closure : String@361
      }
      // bsort_f64@286
      bsort_f64(this.bytes, 0, this.length, f.String) /* fun@286 */;
    }
  }

  // fun@194 (68 ops)
  function splice(len: Int, _: Int): hl.types.ArrayBase {
    if (0 > len) {
      // String@360
      return new hl.types.ArrayBytes_Int();
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
      // String@360
      return new hl.types.ArrayBytes_Int();
    }
    var ret = new hl.types.ArrayBytes_Int();
    ret = new hl.types.ArrayBytes_Int();
    // String@360
    // sub@287
    var v0 = sub(this.bytes, pos << 2, len << 2) /* fun@287 */;
    ret.bytes = v0;
    ret.length = len;
    ret.size = len;
    var end = pos + len;
    // bytes_blit@231
    bytes_blit(this.bytes, pos << 2, this.bytes, end << 2, this.length - end << 2) /* fun@231 */;
    this.length = this.length - len;
    return ret;
  }

  // fun@195 (28 ops)
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
      b.add(this.bytes[i << 2]);
    }
    // nullcheck b
    // addChar@278
    b.addChar(93);
    // toString@279
    var v1 = b.toString();
    return v1;
  }

  // fun@196 (29 ops)
  function unshift(_: Int) {
    if (this.length == this.size) {
      // __expand@218
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // bytes_blit@231
    bytes_blit(this.bytes, 1 << 2, this.bytes, 0, this.length - 1 << 2) /* fun@231 */;
    this.bytes[0 << 2] = x;
  }

  // fun@197 (46 ops)
  function insert(x: Int, _: Int) {
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
      // __expand@218
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // bytes_blit@231
    bytes_blit(this.bytes, pos + 1 << 2, this.bytes, pos << 2, this.length - pos - 1 << 2) /* fun@231 */;
    this.bytes[pos << 2] = x;
  }

  // fun@198 (8 ops)
  function contains(_: Int): Bool {
    // indexOf@200
    var v0 = this.indexOf(x, null);
    if (v0 == -1) {
    }
    return true;
  }

  // fun@199 (27 ops)
  function remove(_: Int): Bool {
    // indexOf@200
    var idx = this.indexOf(x, null);
    if (0 > idx) {
      return false;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    // bytes_blit@231
    bytes_blit(this.bytes, idx << 2, this.bytes, idx + 1 << 2, this.length - idx << 2) /* fun@231 */;
    return true;
  }

  // fun@200 (28 ops)
  function indexOf(fromIndex: Int, _: Null): Int {
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
      if (this.bytes[i << 2] == x) {
        return i;
      }
    }
    return -1;
  }

  // fun@201 (28 ops)
  function lastIndexOf(fromIndex: Int, _: Null): Int {
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
      if (this.bytes[i << 2] == x) {
        return i;
      }
      i--;
    }
    return -1;
  }

  // fun@202 (21 ops)
  function copy(): hl.types.ArrayBytes_Int {
    var a = new hl.types.ArrayBytes_Int();
    a = new hl.types.ArrayBytes_Int();
    // String@360
    a.size = this.length;
    a.length = this.length;
    // alloc_bytes@228
    var v0 = alloc_bytes(this.length << 2) /* fun@228 */;
    a.bytes = v0;
    // bytes_blit@231
    bytes_blit(a.bytes, 0, this.bytes, 0, this.length << 2) /* fun@231 */;
    return a;
  }

  // fun@203 (3 ops)
  function iterator(): haxe.iterators.ArrayIterator {
    // __constructor__@364
    return new hl.types.BytesIterator_Int(this);
  }

  // fun@204 (3 ops)
  function keyValueIterator(): haxe.iterators.ArrayKeyValueIterator {
    // __constructor__@367
    return new hl.types.BytesKeyValueIterator_Int(this);
  }

  // fun@205 (29 ops)
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
      a.array[i] = f(this.bytes[i << 2]);
    }
    // alloc@350
    var v1 = alloc(a, true) /* fun@350 */;
    return v1;
  }

  // fun@206 (19 ops)
  function filter(_: Function): hl.types.ArrayBytes_Int {
    var a = new hl.types.ArrayBytes_Int();
    a = new hl.types.ArrayBytes_Int();
    // String@360
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var v = this.bytes[i << 2];
      // nullcheck f
      if (f(v)) {
        // nullcheck a
        // push@188
        var v1 = a.push(v);
      }
    }
    return a;
  }

  // fun@207 (21 ops)
  function resize(_: Int) {
    if (len > this.length) {
      // __expand@218
      this.__expand(len - 1);
    } else {
      if (this.length <= len) {
        return;
      }
      // bytes_fill@284
      bytes_fill(this.bytes, len << 2, this.length - len << 2, 0) /* fun@284 */;
      this.length = len;
    }
  }

  // fun@208 (12 ops)
  function getDyn(_: Int): Dynamic {
    if (this.length <= pos) {
      return 0;
    }
    return this.bytes[pos << 2];
  }

  // fun@209 (9 ops)
  function setDyn(v: Int, _: Dynamic) {
    if (this.length <= pos) {
      // __expand@218
      this.__expand(pos);
    }
    this.bytes[pos << 2] = v;
  }

  // fun@210 (3 ops)
  function pushDyn(_: Dynamic): Int {
    // push@188
    var v0 = this.push(v);
    return v0;
  }

  // fun@211 (2 ops)
  function popDyn(): Dynamic {
    // pop@187
    var v0 = this.pop();
    return v0;
  }

  // fun@212 (2 ops)
  function shiftDyn(): Dynamic {
    // shift@190
    var v0 = this.shift();
    return v0;
  }

  // fun@213 (3 ops)
  function unshiftDyn(_: Dynamic) {
    // unshift@196
    this.unshift(v);
  }

  // fun@214 (3 ops)
  function insertDyn(v: Int, _: Dynamic) {
    // insert@197
    this.insert(pos, v);
  }

  // fun@215 (3 ops)
  function containsDyn(_: Dynamic): Bool {
    // contains@198
    var v0 = this.contains(v);
    return v0;
  }

  // fun@216 (3 ops)
  function removeDyn(_: Dynamic): Bool {
    // remove@199
    var v0 = this.remove(v);
    return v0;
  }

  // fun@217 (6 ops)
  function sortDyn(_: Function) {
    if (f == null) {
    } else {
      // closure : String@368
    }
    // sort@193
    this.sort(f.String);
  }

  // fun@218 (43 ops)
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
      var bytes2 = alloc_bytes(next << 2) /* fun@228 */;
      var bsize = this.length << 2;
      // bytes_blit@231
      bytes_blit(bytes2, 0, this.bytes, 0, bsize) /* fun@231 */;
      // bytes_fill@284
      bytes_fill(bytes2, bsize, next << 2 - bsize, 0) /* fun@284 */;
      this.bytes = bytes2;
      this.size = next;
    }
    this.length = newlen;
  }
}