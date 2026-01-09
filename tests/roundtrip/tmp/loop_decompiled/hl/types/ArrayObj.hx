package hl.types;

class ArrayObj extends ArrayBase {
  var array: Array<Dynamic>;

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

  // fun@236 (18 ops)
  function concat(_: hl.types.ArrayObj): hl.types.ArrayObj {
    // nullcheck a
    // alloc_array@271
    var arr = alloc_array(Dynamic, this.length + a.length) /* fun@271 */;
    // array_blit@315
    array_blit(arr, 0, this.array, 0, this.length) /* fun@315 */;
    // array_blit@315
    array_blit(arr, this.length, a.array, 0, a.length) /* fun@315 */;
    // String@272
    var v0 = String(arr) /* fun@272 */;
    return v0;
  }

  // fun@237 (20 ops)
  function join(_: String): String {
    var b = new StringBuf();
    b = new StringBuf();
    // __constructor__@276
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      if (i > 0) {
        // nullcheck b
        // add@277
        b.add(sep);
      }
      // nullcheck b
      // add@277
      b.add(this.array[i]);
    }
    // nullcheck b
    // toString@279
    var v1 = b.toString();
    return v1;
  }

  // fun@238 (2 ops)
  function isArrayObj(): Bool {
    return true;
  }

  // fun@239 (16 ops)
  function pop(): Dynamic {
    if (this.length == 0) {
      return null;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    var v = this.array[this.length];
    this.array[this.length] = null;
    return v;
  }

  // fun@240 (13 ops)
  function push(_: Dynamic): Int {
    var len = this.length;
    if (this.array.length == len) {
      // __expand@260
      this.__expand(len);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    this.array[len] = x;
    return this.length;
  }

  // fun@241 (22 ops)
  function reverse() {
    while (this.length >> 1 > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var k = this.length - 1 - i;
      var tmp = this.array[i];
      this.array[i] = this.array[k];
      this.array[k] = tmp;
    }
  }

  // fun@242 (22 ops)
  function shift(): Dynamic {
    if (this.length == 0) {
      return null;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    var v = this.array[0];
    // array_blit@315
    array_blit(this.array, 0, this.array, 1, this.length) /* fun@315 */;
    this.array[this.length] = null;
    return v;
  }

  // fun@243 (33 ops)
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
      // String@300
      return new hl.types.ArrayObj();
    }
    // alloc_array@271
    var v0 = alloc_array(Dynamic, len) /* fun@271 */;
    // array_blit@315
    array_blit(v0, 0, this.array, pos, len) /* fun@315 */;
    // String@272
    var v1 = String(v0) /* fun@272 */;
    return v1;
  }

  // fun@244 (3 ops)
  function sort(_: Function) {
    // sort@316
    sort(this, f) /* fun@316 */;
  }

  // fun@245 (54 ops)
  function splice(len: Int, _: Int): hl.types.ArrayBase {
    if (0 > len) {
      // String@300
      return new hl.types.ArrayObj();
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
    var a = this.array;
    // alloc_array@271
    var v0 = alloc_array(Dynamic, len) /* fun@271 */;
    // array_blit@315
    array_blit(v0, 0, a, pos, len) /* fun@315 */;
    // String@272
    var ret = String(v0) /* fun@272 */;
    var end = pos + len;
    // array_blit@315
    array_blit(a, pos, a, end, this.length - end) /* fun@315 */;
    this.length = this.length - len;
    while (0 <= len) {
      len--;
      a[this.length + len] = null;
    }
    return ret;
  }

  // fun@246 (51 ops)
  function toString(): String {
    if (5 <= $Std.toStringDepth) {
      return "...";
    }
    var v0 = $Std.toStringDepth;
    v0++;
    $Std.toStringDepth = v0;
    var b = new StringBuf();
    b = new StringBuf();
    // __constructor__@276
    // addChar@278
    b.addChar(91);
    try {
      while (this.length > 0) {
        var i = 0;
        var v1 = 0;
        v1++;
        if (i > 0) {
          // nullcheck b
          // addChar@278
          b.addChar(44);
        }
        // nullcheck b
        // add@277
        b.add(this.array[i]);
      }
    } catch (e) {
      // caught@303
      var v2 = caught(e) /* fun@303 */;
      // nullcheck v2
      var e = v2.__exceptionMessage();
      var v3 = $Std.toStringDepth;
      v3--;
      $Std.toStringDepth = v3;
      throw e;
    }
    // nullcheck b
    // addChar@278
    b.addChar(93);
    var v4 = $Std.toStringDepth;
    v4--;
    $Std.toStringDepth = v4;
    // toString@279
    var v5 = b.toString();
    return v5;
  }

  // fun@247 (22 ops)
  function unshift(_: Dynamic) {
    if (this.length == this.array.length) {
      // __expand@260
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // array_blit@315
    array_blit(this.array, 1, this.array, 0, this.length - 1) /* fun@315 */;
    this.array[0] = x;
  }

  // fun@248 (36 ops)
  function insert(x: Int, _: Dynamic) {
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
    if (this.length == this.array.length) {
      // __expand@260
      this.__expand(this.length);
    } else {
      var v0 = this.length;
      v0++;
      this.length = v0;
    }
    // array_blit@315
    array_blit(this.array, pos + 1, this.array, pos, this.length - pos - 1) /* fun@315 */;
    this.array[pos] = x;
  }

  // fun@249 (8 ops)
  function contains(_: Dynamic): Bool {
    // indexOf@251
    var v0 = this.indexOf(x, null);
    if (v0 == -1) {
    }
    return true;
  }

  // fun@250 (22 ops)
  function remove(_: Dynamic): Bool {
    // indexOf@251
    var i = this.indexOf(x, null);
    if (0 > i) {
      return false;
    }
    var v0 = this.length;
    v0--;
    this.length = v0;
    // array_blit@315
    array_blit(this.array, i, this.array, i + 1, this.length - i) /* fun@315 */;
    this.array[this.length] = null;
    return true;
  }

  // fun@251 (21 ops)
  function indexOf(fromIndex: Dynamic, _: Null): Int {
    var i = fromIndex;
    if (0 > i) {
      i = i + this.length;
      if (0 > i + this.length) {
        i = 0;
      }
    }
    var length = this.length;
    var array = this.array;
    while (length > i) {
      if (array[i] == x) {
        return i;
      }
      i++;
    }
    return -1;
  }

  // fun@252 (21 ops)
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
    // array_blit@315
    array_blit(this.array, pos, src.array, srcpos, len) /* fun@315 */;
  }

  // fun@253 (26 ops)
  function lastIndexOf(fromIndex: Dynamic, _: Null): Int {
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
      if (this.array[i] == x) {
        return i;
      }
      i--;
    }
    return -1;
  }

  // fun@254 (10 ops)
  function copy(): hl.types.ArrayObj {
    // alloc_array@271
    var n = alloc_array(Dynamic, this.length) /* fun@271 */;
    // array_blit@315
    array_blit(n, 0, this.array, 0, this.length) /* fun@315 */;
    // String@272
    var v0 = String(n) /* fun@272 */;
    return v0;
  }

  // fun@255 (3 ops)
  function iterator(): haxe.iterators.ArrayIterator {
    // __constructor__@393
    return new hl.types.ArrayObjIterator(this);
  }

  // fun@256 (3 ops)
  function keyValueIterator(): haxe.iterators.ArrayKeyValueIterator {
    // __constructor__@396
    return new hl.types.ArrayObjKeyValueIterator(this);
  }

  // fun@257 (27 ops)
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
      a.array[i] = f(this.array[i]);
    }
    // alloc@350
    var v1 = alloc(a, true) /* fun@350 */;
    return v1;
  }

  // fun@258 (17 ops)
  function filter(_: Function): hl.types.ArrayObj {
    var a = new hl.types.ArrayObj();
    a = new hl.types.ArrayObj();
    // String@300
    while (this.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      var v = this.array[i];
      // nullcheck f
      if (f(v)) {
        // nullcheck a
        // push@240
        var v1 = a.push(v);
      }
    }
    return a;
  }

  // fun@259 (20 ops)
  function resize(_: Int) {
    if (len > this.length) {
      // __expand@260
      this.__expand(len - 1);
    } else {
      if (this.length <= len) {
        return;
      }
      while (len > this.length) {
        var i = this.length;
        var v0 = this.length;
        v0++;
        this.array[i] = null;
      }
      this.length = len;
    }
  }

  // fun@260 (31 ops)
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
    var size = this.array.length;
    if (newlen > size) {
      var next = size * 3 >> 1;
      if (newlen > next) {
        next = newlen;
      }
      // alloc_array@271
      var arr2 = alloc_array(Dynamic, next) /* fun@271 */;
      // array_blit@315
      array_blit(arr2, 0, this.array, 0, this.length) /* fun@315 */;
      this.array = arr2;
    }
    this.length = newlen;
  }

  // fun@261 (7 ops)
  function getDyn(_: Int): Dynamic {
    if (this.length <= pos) {
      return null;
    }
    return this.array[pos];
  }

  // fun@262 (9 ops)
  function setDyn(v: Int, _: Dynamic) {
    if (this.length <= pos) {
      // __expand@260
      this.__expand(pos);
    }
    // array_type@343
    var v0 = array_type(this.array) /* fun@343 */;
    // value_cast@336
    var v1 = value_cast(v, v0) /* fun@336 */;
    this.array[pos] = v1;
  }

  // fun@263 (2 ops)
  function pushDyn(_: Dynamic): Int {
    // push@240
    var v0 = this.push(v);
    return v0;
  }

  // fun@264 (2 ops)
  function popDyn(): Dynamic {
    // pop@239
    var v0 = this.pop();
    return v0;
  }

  // fun@265 (2 ops)
  function shiftDyn(): Dynamic {
    // shift@242
    var v0 = this.shift();
    return v0;
  }

  // fun@266 (2 ops)
  function unshiftDyn(_: Dynamic) {
    // unshift@247
    this.unshift(v);
  }

  // fun@267 (2 ops)
  function insertDyn(v: Int, _: Dynamic) {
    // insert@248
    this.insert(pos, v);
  }

  // fun@268 (2 ops)
  function containsDyn(_: Dynamic): Bool {
    // contains@249
    var v0 = this.contains(v);
    return v0;
  }

  // fun@269 (2 ops)
  function removeDyn(_: Dynamic): Bool {
    // remove@250
    var v0 = this.remove(v);
    return v0;
  }

  // fun@270 (2 ops)
  function sortDyn(_: Function) {
    // sort@244
    this.sort(f);
  }
}