package hl.types;

class ArrayDyn extends ArrayAccess {
  var array: hl.types.ArrayBase;
  var allowReinterpret: Bool;

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

  // fun@31 (4 ops)
  function get_length(): Int {
    // nullcheck this.array
    return this.array.length;
  }

  // fun@32 (4 ops)
  function getDyn(_: Int): Dynamic {
    // nullcheck this.array
    return this.array.length(i);
  }

  // fun@33 (4 ops)
  function setDyn(value: Int, _: Dynamic) {
    // nullcheck this.array
    this.array.<field1>(pos, value);
  }

  // fun@34 (4 ops)
  function blit(src: Int, srcpos: hl.types.ArrayAccess, len: Int, _: Int) {
    // nullcheck this.array
    this.array.<field2>(pos, src, srcpos, len);
  }

  // fun@35 (38 ops)
  function concat(_: hl.types.ArrayDyn): hl.types.ArrayDyn {
    var a1 = this.array;
    // nullcheck a
    var a2 = a.array;
    // nullcheck a1
    var alen = a1.length;
    // nullcheck a2
    // alloc_array@271
    var v0 = alloc_array(Dynamic, alen + a2.length) /* fun@271 */;
    var anew = v0;
    while (alen > 0) {
      var i = 0;
      var v1 = 0;
      v1++;
      // nullcheck a1
      anew[i] = a1.length(i);
    }
    // nullcheck a2
    while (a2.length > 0) {
      i = 0;
      var v2 = 0;
      v2++;
      // nullcheck a2
      anew[i + alen] = a2.length(i);
    }
    // String@272
    var v3 = String(anew) /* fun@272 */;
    // alloc@350
    var v4 = alloc(v3, true) /* fun@350 */;
    return v4;
  }

  // fun@36 (4 ops)
  function join(_: String): String {
    // nullcheck this.array
    return this.array.<field13>(sep);
  }

  // fun@37 (4 ops)
  function pop(): Dynamic {
    // nullcheck this.array
    return this.array.<field4>();
  }

  // fun@38 (4 ops)
  function push(_: Dynamic): Int {
    // nullcheck this.array
    return this.array.<field3>(x);
  }

  // fun@39 (4 ops)
  function reverse() {
    // nullcheck this.array
    this.array.<field14>();
  }

  // fun@40 (4 ops)
  function resize(_: Int) {
    // nullcheck this.array
    this.array.<field15>(len);
  }

  // fun@41 (4 ops)
  function shift(): Dynamic {
    // nullcheck this.array
    return this.array.<field5>();
  }

  // fun@42 (7 ops)
  function slice(end: Int, _: Null): hl.types.ArrayDyn {
    // nullcheck this.array
    // alloc@350
    var v0 = alloc(this.array.<field11>(pos, end), true) /* fun@350 */;
    return v0;
  }

  // fun@43 (4 ops)
  function sort(_: Function) {
    // nullcheck this.array
    this.array.<field10>(f);
  }

  // fun@44 (7 ops)
  function splice(len: Int, _: Int): hl.types.ArrayDyn {
    // nullcheck this.array
    // alloc@350
    var v0 = alloc(this.array.<field12>(pos, len), true) /* fun@350 */;
    return v0;
  }

  // fun@45 (4 ops)
  function toString(): String {
    // nullcheck this.array
    return this.array.<field16>();
  }

  // fun@46 (4 ops)
  function unshift(_: Dynamic) {
    // nullcheck this.array
    this.array.<field6>(x);
  }

  // fun@47 (4 ops)
  function insert(x: Int, _: Dynamic) {
    // nullcheck this.array
    this.array.<field7>(pos, x);
  }

  // fun@48 (4 ops)
  function contains(_: Dynamic): Bool {
    // nullcheck this.array
    return this.array.<field8>(x);
  }

  // fun@49 (4 ops)
  function remove(_: Dynamic): Bool {
    // nullcheck this.array
    return this.array.<field9>(x);
  }

  // fun@50 (15 ops)
  function indexOf(fromIndex: Dynamic, _: Null): Int {
    var i = fromIndex;
    // nullcheck this.array
    var length = this.array.length;
    var array = this.array;
    while (length > i) {
      // nullcheck array
      if (array.length(i) == x) {
        return i;
      }
      i++;
    }
    return -1;
  }

  // fun@51 (29 ops)
  function lastIndexOf(fromIndex: Dynamic, _: Null): Int {
    // nullcheck this.array
    var len = this.array.length;
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
      // nullcheck this.array
      if (this.array.length(i) == x) {
        return i;
      }
      i--;
    }
    return -1;
  }

  // fun@52 (24 ops)
  function copy(): hl.types.ArrayDyn {
    // nullcheck this.array
    // alloc_array@271
    var v0 = alloc_array(Dynamic, this.array.length) /* fun@271 */;
    var a = v0;
    // nullcheck this.array
    while (this.array.length > 0) {
      var i = 0;
      var v1 = 0;
      v1++;
      // nullcheck this.array
      a[i] = this.array.length(i);
    }
    // String@272
    var v2 = String(a) /* fun@272 */;
    // alloc@350
    var v3 = alloc(v2, true) /* fun@350 */;
    return v3;
  }

  // fun@53 (4 ops)
  function iterator(): haxe.iterators.ArrayIterator {
    // __constructor__@388
    return new hl.types.ArrayDynIterator(this.array);
  }

  // fun@54 (4 ops)
  function keyValueIterator(): haxe.iterators.ArrayKeyValueIterator {
    // __constructor__@391
    return new hl.types.ArrayDynKeyValueIterator(this.array);
  }

  // fun@55 (26 ops)
  function map(_: Function): hl.types.ArrayDyn {
    // nullcheck this.array
    // alloc_array@271
    var v0 = alloc_array(Dynamic, this.array.length) /* fun@271 */;
    var a = v0;
    // nullcheck this.array
    while (this.array.length > 0) {
      var i = 0;
      var v1 = 0;
      v1++;
      // nullcheck f
      // nullcheck this.array
      a[i] = f(this.array.length(i));
    }
    // String@272
    var v2 = String(a) /* fun@272 */;
    // alloc@350
    var v3 = alloc(v2, true) /* fun@350 */;
    return v3;
  }

  // fun@56 (23 ops)
  function filter(_: Function): hl.types.ArrayDyn {
    var a = new hl.types.ArrayObj();
    a = new hl.types.ArrayObj();
    // String@300
    // nullcheck this.array
    while (this.array.length > 0) {
      var i = 0;
      var v0 = 0;
      v0++;
      // nullcheck this.array
      var v = this.array.length(i);
      // nullcheck f
      if (f(v)) {
        // nullcheck a
        // push@240
        var v1 = a.push(v);
      }
    }
    // alloc@350
    var v2 = alloc(a, true) /* fun@350 */;
    return v2;
  }

  // fun@57 (9 ops)
  function __get_field(_: Int): Dynamic {
    if (fid == -16280745) {
      // nullcheck this.array
      return this.array.length;
    }
    return null;
  }

  // fun@58 (81 ops)
  function __cast(_: Class<Dynamic>): Dynamic {
    if (t == typeof(this.array)) {
      return this.array;
    }
    if (!this.allowReinterpret) {
      return null;
    }
    if (t == hl.types.ArrayBytes_Int) {
      var a = null;
      // nullcheck this.array
      // alloc_bytes@228
      var v0 = alloc_bytes(this.array.length << 2) /* fun@228 */;
      a = v0;
      // nullcheck this.array
      while (this.array.length > 0) {
        var i = 0;
        var v1 = 0;
        v1++;
        // nullcheck this.array
        a[i << 2] = this.array.length(i);
      }
      // nullcheck this.array
      // allocI32@78
      var arr = allocI32(a, this.array.length) /* fun@78 */;
      this.array = arr;
      this.allowReinterpret = false;
      return arr;
    }
    if (t == hl.types.ArrayBytes_Float) {
      a = null;
      // nullcheck this.array
      // alloc_bytes@228
      var v2 = alloc_bytes(this.array.length << 3) /* fun@228 */;
      a = v2;
      // nullcheck this.array
      while (this.array.length > 0) {
        i = 0;
        var v3 = 0;
        v3++;
        // nullcheck this.array
        a[i << 3] = this.array.length(i);
      }
      // nullcheck this.array
      // allocF64@81
      arr = allocF64(a, this.array.length) /* fun@81 */;
      this.array = arr;
      this.allowReinterpret = false;
      return arr;
    }
    return null;
  }

  // fun@59 (6 ops)
  function __compare(_: Dynamic): Int {
    if (a == this.array) {
      return 0;
    }
    // ptr_compare@274
    var v0 = ptr_compare(this, a) /* fun@274 */;
    return v0;
  }

  // fun@60 (4 ops)
  function __string(): hl.Bytes {
    // toString@45
    var v0 = this.toString();
    // nullcheck v0
    return v0.bytes;
  }
}