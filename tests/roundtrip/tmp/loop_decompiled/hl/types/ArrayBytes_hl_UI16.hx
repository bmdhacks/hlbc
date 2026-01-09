package hl.types;

class ArrayBytes_hl_UI16 extends ArrayBase {
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

  // fun@151 (32 ops)
  function concat(_: hl.types.ArrayBytes_hl_UI16): hl.types.ArrayBytes_hl_UI16 {
    var ac = new hl.types.ArrayBytes_hl_UI16();
    ac = new hl.types.ArrayBytes_hl_UI16();
    // String@378
    // nullcheck a
    ac.size = this.length + a.length;
    ac.length = this.length + a.length;
    // alloc_bytes@228
    var v0 = alloc_bytes(ac.length << 1) /* fun@228 */;
    ac.bytes = v0;
    var offset = this.length << 1;
    // bytes_blit@231
    bytes_blit(ac.bytes, 0, this.bytes, 0, offset) /* fun@231 */;
    // bytes_blit@231
    bytes_blit(ac.bytes, offset, a.bytes, 0, a.length << 1) /* fun@231 */;
    return ac;
  }

  // fun@152 (24 ops)
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
      s.add(this.bytes[i << 1]);
    }
    // nullcheck s
    // toString@279
    var v1 = s.toString();
    return v1;
  }

  // fun@153 (16 ops)
  function pop(): Null {
    if (this.length == 0) {
      return null;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    return this.bytes[this.length << 1];
  }

  // fun@154 (15 ops)
  function push(_: hl.UI16): Int {
    var len = this.length;
    if (this.size == len) {
      // __expand@184
      this.__expand(len);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    this.bytes[len << 1] = x;
    return this.length;
  }

  // fun@155 (34 ops)
  function reverse() {
    while (this.length >> 1 > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var k = this.length - 1 - i;
      var tmp = this.bytes[i << 1];
      this.bytes[i << 1] = this.bytes[k << 1];
      this.bytes[k << 1] = tmp;
    }
  }

  // fun@156 (28 ops)
  function shift(): Null {
    if (this.length == 0) {
      return null;
    }
    var v = this.bytes[0 << 1];
    var v0 = this.length;
    v0--;
    this.length = v0;
    // bytes_blit@231
    bytes_blit(this.bytes, 0, this.bytes, 1 << 1, this.length << 1) /* fun@231 */;
    return v;
  }

  // fun@157 (30 ops)
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
    bytes_blit(this.bytes, pos << 1, src.bytes, srcpos << 1, len << 1) /* fun@231 */;
  }

  // fun@158 (40 ops)
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
      // String@378
      return new hl.types.ArrayBytes_hl_UI16();
    }
    var a = new hl.types.ArrayBytes_hl_UI16();
    a = new hl.types.ArrayBytes_hl_UI16();
    // String@378
    a.size = len;
    a.length = len;
    // sub@287
    var v0 = sub(this.bytes, pos << 1, len << 1) /* fun@287 */;
    a.bytes = v0;
    return a;
  }

  // fun@159 (21 ops)
  function sort(_: Function) {
    if (Int == Int) {
      if (f == null) {
      } else {
        // closure : String@379
      }
      // bsort_i32@285
      bsort_i32(this.bytes, 0, this.length, f.String) /* fun@285 */;
    } else {
      if (f == null) {
      } else {
        // closure : String@380
      }
      // bsort_f64@286
      bsort_f64(this.bytes, 0, this.length, f.String) /* fun@286 */;
    }
  }

  // fun@160 (68 ops)
  function splice(len: Int, _: Int): hl.types.ArrayBase {
    if (0 > len) {
      // String@378
      return new hl.types.ArrayBytes_hl_UI16();
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
      // String@378
      return new hl.types.ArrayBytes_hl_UI16();
    }
    var ret = new hl.types.ArrayBytes_hl_UI16();
    ret = new hl.types.ArrayBytes_hl_UI16();
    // String@378
    // sub@287
    var v0 = sub(this.bytes, pos << 1, len << 1) /* fun@287 */;
    ret.bytes = v0;
    ret.length = len;
    ret.size = len;
    var end = pos + len;
    // bytes_blit@231
    bytes_blit(this.bytes, pos << 1, this.bytes, end << 1, this.length - end << 1) /* fun@231 */;
    this.length = this.length - len;
    return ret;
  }

  // fun@161 (29 ops)
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
      b.add(this.bytes[i << 1]);
    }
    // nullcheck b
    // addChar@278
    b.addChar(93);
    // toString@279
    var v1 = b.toString();
    return v1;
  }

  // fun@162 (30 ops)
  function unshift(_: hl.UI16) {
    if (this.length == this.size) {
      // __expand@184
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // bytes_blit@231
    bytes_blit(this.bytes, 1 << 1, this.bytes, 0, this.length - 1 << 1) /* fun@231 */;
    this.bytes[0 << 1] = x;
  }

  // fun@163 (47 ops)
  function insert(x: Int, _: hl.UI16) {
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
      // __expand@184
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // bytes_blit@231
    bytes_blit(this.bytes, pos + 1 << 1, this.bytes, pos << 1, this.length - pos - 1 << 1) /* fun@231 */;
    this.bytes[pos << 1] = x;
  }

  // fun@164 (8 ops)
  function contains(_: hl.UI16): Bool {
    // indexOf@166
    var v0 = this.indexOf(x, null);
    if (v0 == -1) {
    }
    return true;
  }

  // fun@165 (27 ops)
  function remove(_: hl.UI16): Bool {
    // indexOf@166
    var idx = this.indexOf(x, null);
    if (0 > idx) {
      return false;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    // bytes_blit@231
    bytes_blit(this.bytes, idx << 1, this.bytes, idx + 1 << 1, this.length - idx << 1) /* fun@231 */;
    return true;
  }

  // fun@166 (29 ops)
  function indexOf(fromIndex: hl.UI16, _: Null): Int {
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
      if (this.bytes[i << 1] == x) {
        return i;
      }
    }
    return -1;
  }

  // fun@167 (29 ops)
  function lastIndexOf(fromIndex: hl.UI16, _: Null): Int {
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
      if (this.bytes[i << 1] == x) {
        return i;
      }
      i--;
    }
    return -1;
  }

  // fun@168 (21 ops)
  function copy(): hl.types.ArrayBytes_hl_UI16 {
    var a = new hl.types.ArrayBytes_hl_UI16();
    a = new hl.types.ArrayBytes_hl_UI16();
    // String@378
    a.size = this.length;
    a.length = this.length;
    // alloc_bytes@228
    var v0 = alloc_bytes(this.length << 1) /* fun@228 */;
    a.bytes = v0;
    // bytes_blit@231
    bytes_blit(a.bytes, 0, this.bytes, 0, this.length << 1) /* fun@231 */;
    return a;
  }

  // fun@169 (3 ops)
  function iterator(): haxe.iterators.ArrayIterator {
    // __constructor__@383
    return new hl.types.BytesIterator_hl_UI16(this);
  }

  // fun@170 (3 ops)
  function keyValueIterator(): haxe.iterators.ArrayKeyValueIterator {
    // __constructor__@386
    return new hl.types.BytesKeyValueIterator_hl_UI16(this);
  }

  // fun@171 (30 ops)
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
      a.array[i] = f(this.bytes[i << 1]);
    }
    // alloc@350
    var v1 = alloc(a, true) /* fun@350 */;
    return v1;
  }

  // fun@172 (20 ops)
  function filter(_: Function): hl.types.ArrayBytes_hl_UI16 {
    var a = new hl.types.ArrayBytes_hl_UI16();
    a = new hl.types.ArrayBytes_hl_UI16();
    // String@378
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var v = this.bytes[i << 1];
      // nullcheck f
      if (f(v)) {
        // nullcheck a
        // push@154
        var v1 = a.push(v);
      }
    }
    return a;
  }

  // fun@173 (21 ops)
  function resize(_: Int) {
    if (len > this.length) {
      // __expand@184
      this.__expand(len - 1);
    } else {
      if (this.length <= len) {
        return;
      }
      // bytes_fill@284
      bytes_fill(this.bytes, len << 1, this.length - len << 1, 0) /* fun@284 */;
      this.length = len;
    }
  }

  // fun@174 (12 ops)
  function getDyn(_: Int): Dynamic {
    if (this.length <= pos) {
      return 0;
    }
    return this.bytes[pos << 1];
  }

  // fun@175 (10 ops)
  function setDyn(v: Int, _: Dynamic) {
    if (this.length <= pos) {
      // __expand@184
      this.__expand(pos);
    }
    this.bytes[pos << 1] = v;
  }

  // fun@176 (3 ops)
  function pushDyn(_: Dynamic): Int {
    // push@154
    var v0 = this.push(v);
    return v0;
  }

  // fun@177 (2 ops)
  function popDyn(): Dynamic {
    // pop@153
    var v0 = this.pop();
    return v0;
  }

  // fun@178 (2 ops)
  function shiftDyn(): Dynamic {
    // shift@156
    var v0 = this.shift();
    return v0;
  }

  // fun@179 (3 ops)
  function unshiftDyn(_: Dynamic) {
    // unshift@162
    this.unshift(v);
  }

  // fun@180 (3 ops)
  function insertDyn(v: Int, _: Dynamic) {
    // insert@163
    this.insert(pos, v);
  }

  // fun@181 (3 ops)
  function containsDyn(_: Dynamic): Bool {
    // contains@164
    var v0 = this.contains(v);
    return v0;
  }

  // fun@182 (3 ops)
  function removeDyn(_: Dynamic): Bool {
    // remove@165
    var v0 = this.remove(v);
    return v0;
  }

  // fun@183 (6 ops)
  function sortDyn(_: Function) {
    if (f == null) {
    } else {
      // closure : String@387
    }
    // sort@159
    this.sort(f.String);
  }

  // fun@184 (43 ops)
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
      var bytes2 = alloc_bytes(next << 1) /* fun@228 */;
      var bsize = this.length << 1;
      // bytes_blit@231
      bytes_blit(bytes2, 0, this.bytes, 0, bsize) /* fun@231 */;
      // bytes_fill@284
      bytes_fill(bytes2, bsize, next << 1 - bsize, 0) /* fun@284 */;
      this.bytes = bytes2;
      this.size = next;
    }
    this.length = newlen;
  }
}