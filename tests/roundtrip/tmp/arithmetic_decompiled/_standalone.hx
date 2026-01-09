// Standalone functions (not part of any class)

// fun@0
// fun@22 (109 ops)
static function main() {
  var a = 10;
  var b = 3;
  var sum = a + b;
  var diff = a - b;
  var prod = a * b;
  var quot = a / b;
  var mod = a % b;
  // nullcheck haxe.$Log.trace
  // itos@216
  var v1 = sum;
  // __alloc__@16
  var v2 = v1;
  // __add__@20
  var v3 = "sum=" + v2;
  trace(v3) /* fun@215 */;
  // nullcheck haxe.$Log.trace
  // itos@216
  var v5 = diff;
  // __alloc__@16
  var v6 = v5;
  // __add__@20
  var v7 = "diff=" + v6;
  trace(v7) /* fun@215 */;
  // nullcheck haxe.$Log.trace
  // itos@216
  var v9 = prod;
  // __alloc__@16
  var v10 = v9;
  // __add__@20
  var v11 = "prod=" + v10;
  trace(v11) /* fun@215 */;
  // nullcheck haxe.$Log.trace
  // ftos@217
  var v12 = quot;
  // __alloc__@16
  var v13 = v12;
  // __add__@20
  var v14 = "quot=" + v13;
  trace(v14) /* fun@215 */;
  // nullcheck haxe.$Log.trace
  // itos@216
  var v16 = mod;
  // __alloc__@16
  var v17 = v16;
  // __add__@20
  var v18 = "mod=" + v17;
  trace(v18) /* fun@215 */;
}

// fun@1
// fun@220 (3 ops)
static function __constructor__(year: Date, month: Int, day: Int, hour: Int, min: Int, sec: Int, _: Int) {
  // date_new@218
  var v0 = date_new(year, month, day, hour, min, sec) /* fun@218 */;
  this.t = v0;
}

// fun@4
// fun@224 (37 ops)
static function isOfType(v: Dynamic, t: Dynamic): Bool {
  t = t;
  if (t == null) {
    return false;
  }
  // nullcheck t
  switch (typeid(t.__type__)) {
    case 3:
      switch (typeid(typeof(v))) {
        case 5:
          v = v;
          if (v != v) {
          }
          return true;
      }
    case 6:
      switch (typeid(typeof(v))) {
        case 1:
          return true;
      }
    case 9:
      if (v == null) {
      }
      return true;
  }
  // check@14
  var v0 = t.check(v);
  return v0;
}

// fun@5
// fun@225 (7 ops)
static function string(s: Dynamic): String {
  var len = 0;
  // value_to_string@226
  var bytes = value_to_string(s, len) /* fun@226 */;
  return new String();
}

// fun@6
// fun@227 (77 ops)
static function __add__(a: Dynamic, b: Dynamic): Dynamic {
  var ta = typeof(a);
  var tb = typeof(b);
  if (ta == String) {
    // string@225
    var v0 = string(b) /* fun@225 */;
    // __add__@20
    var v1 = a + v0;
    return v1;
  }
  if (tb == String) {
    // string@225
    var v2 = string(a) /* fun@225 */;
    // __add__@20
    var v3 = v2 + b;
    return v3;
  }
  switch (typeid(ta)) {
    default:
      switch (typeid(tb)) {
        case 0:
        case 1:
          return a;
        case 5:
          return a + b;
          return a + b;
      }
    case 0:
      switch (typeid(tb)) {
        case 0:
        case 1:
          return 0;
          return b;
      }
    case 1:
      a = a;
      switch (typeid(tb)) {
        case 0:
        case 1:
          return a;
        case 5:
          return a + b;
          return a + b;
      }
    case 5:
      a = a;
  }
  // string@225
  var v4 = string(a) /* fun@225 */;
  // __add__@20
  var v5 = "Can't add " + v4;
  // __add__@20
  var v6 = v5 + "(";
  // string@225
  var v7 = string(ta) /* fun@225 */;
  // __add__@20
  var v8 = v6 + v7;
  // __add__@20
  var v9 = v8 + ") and ";
  // string@225
  var v10 = string(b) /* fun@225 */;
  // __add__@20
  var v11 = v9 + v10;
  // __add__@20
  var v12 = v11 + "(";
  // string@225
  var v13 = string(tb) /* fun@225 */;
  // __add__@20
  var v14 = v12 + v13;
  // __add__@20
  var v15 = v14 + ")";
  // thrown@228
  var v16 = v15;
  throw v16;
}

// fun@7
// fun@15 (63 ops)
static function fromCharCode(code: Int): String {
  if (0 <= code) {
    if (65536 > code) {
      if (55296 <= code) {
        if (code <= 57343) {
          // itos@216
          var v1 = code;
          // __alloc__@16
          var v2 = v1;
          // __add__@20
          var v3 = "Invalid unicode char " + v2;
          // thrown@228
          var v4 = v3;
          throw v4;
        }
      }
      // alloc_bytes@229
      var b = alloc_bytes(4) /* fun@229 */;
      b[0] = v0;
      b[2] = 0;
      return new String();
    }
  }
  if (1114112 > v0) {
    // alloc_bytes@229
    b = alloc_bytes(6) /* fun@229 */;
    code = v0 - 65536;
    b[0] = code >> 10 + 55296;
    b[2] = code && 1023 + 56320;
    b[4] = 0;
    return new String();
  }
  // itos@216
  var v6 = code;
  // __alloc__@16
  var v7 = v6;
  // thrown@228
  throw new String();
}

// fun@8
// fun@16 (4 ops)
static function __alloc__(b: hl.Bytes, length: Int): String {
  var s = new String();
  return s;
}

// fun@9
// fun@17 (8 ops)
static function call_toString(v: Dynamic): hl.Bytes {
  // nullcheck v
  // nullcheck r1
  var s = v["toString"]();
  // nullcheck s
  return s.bytes;
}

// fun@10
// fun@18 (6 ops)
static function fromUCS2(b: hl.Bytes): String {
  var s = new String();
  // ucs2length@230
  var v0 = ucs2length(b, 0) /* fun@230 */;
  return s;
}

// fun@11
// fun@19 (10 ops)
static function fromUTF8(b: hl.Bytes): String {
  var outLen = 0;
  // utf8_to_utf16@231
  var b2 = utf8_to_utf16(b, 0, outLen) /* fun@231 */;
  return new String();
}

// fun@12
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
  // alloc_bytes@229
  var bytes = alloc_bytes(tot + 2) /* fun@229 */;
  // bytes_blit@232
  bytes_blit(bytes, 0, a.bytes, 0, asize) /* fun@232 */;
  // bytes_blit@232
  bytes_blit(bytes, asize, b.bytes, 0, bsize) /* fun@232 */;
  bytes[tot] = 0;
  return new String();
}

// fun@13
// fun@21 (6 ops)
static function __constructor__(string: String, _: String) {
  // nullcheck string
  this.bytes = string.bytes;
  this.length = string.length;
}

// fun@28
// fun@277 (8 ops)
static function __constructor__(_: StringBuf) {
  this.pos = 0;
  this.size = 8;
  // alloc_bytes@229
  var v0 = alloc_bytes(this.size) /* fun@229 */;
  this.b = v0;
}

// fun@33
// fun@282 (2 ops)
static function __constructor__(msg: SysError, _: String) {
  this.msg = msg;
}

// fun@36
// fun@288 (4 ops)
static function sub(this1: hl.Bytes, pos: Int, size: Int): hl.Bytes {
  // alloc_bytes@229
  var b = alloc_bytes(size) /* fun@229 */;
  // bytes_blit@232
  bytes_blit(b, 0, this1, pos, size) /* fun@232 */;
  return b;
}

// fun@37
// fun@289 (9 ops)
static function println(v: Dynamic) {
  // string@225
  var v0 = string(v) /* fun@225 */;
  // nullcheck v0
  // sys_print@290
  sys_print(v0.bytes) /* fun@290 */;
  // nullcheck r3
  // sys_print@290
  sys_print("
".bytes) /* fun@290 */;
}

// fun@38
// fun@292 (3 ops)
static function init() {
  @G27 = hballoc() /* fun@293 */;
}

// fun@39
// fun@294 (13 ops)
static function initClass(ct: Class<Dynamic>, t: Class<Dynamic>, name: hl.Bytes): hl.Class {
  // alloc_obj@295
  var v0 = alloc_obj(ct) /* fun@295 */;
  var c = v0;
  // type_set_global@296
  var v1 = type_set_global(t, c) /* fun@296 */;
  // nullcheck c
  c.__type__ = t;
  // ucs2length@230
  var v2 = ucs2length(name, 0) /* fun@230 */;
  // register@297
  register(name, c) /* fun@297 */;
  return c;
}

// fun@40
// fun@298 (49 ops)
static function initEnum(et: Class<Dynamic>, t: Class<Dynamic>): hl.Enum {
  // alloc_obj@295
  var v0 = alloc_obj(et) /* fun@295 */;
  var e = v0;
  // nullcheck e
  e.__type__ = t;
  // type_enum_values@299
  var v1 = type_enum_values(t) /* fun@299 */;
  e.__evalues__ = v1;
  // type_name@300
  var v2 = type_name(t) /* fun@300 */;
  if (v2 == null) {
  } else {
    // ucs2length@230
    var v3 = ucs2length(v2, 0) /* fun@230 */;
  }
  // String@301
  // type_enum_fields@302
  var cl = type_enum_fields(t) /* fun@302 */;
  while (true) {
    if (cl.length > 0) {
      var i = 0;
      i++;
      var name = cl[i];
      // nullcheck e
      // hbset@303
      hbset(e.__emap__, name, i) /* fun@303 */;
      // ucs2length@230
      var v4 = ucs2length(name, 0) /* fun@230 */;
      // nullcheck e.__constructs__
      // push@241
      var v5 = e.__constructs__.push(new String());
    }
  }
  // nullcheck e
  // nullcheck e.__ename__
  // register@297
  register(e.__ename__.bytes, e) /* fun@297 */;
  // type_set_global@296
  var v6 = type_set_global(t, e) /* fun@296 */;
  return e;
}

// fun@41
// fun@297 (3 ops)
static function register(b: hl.Bytes, t: hl.BaseType) {
  // hbset@303
  hbset(@G27, b, t) /* fun@303 */;
}

// fun@42
// fun@304 (9 ops)
static function caught(value: Dynamic): haxe.Exception {
  // isOfType@224
  var v0 = isOfType(value, haxe.$Exception) /* fun@224 */;
  if (v0) {
    return value;
  }
  // __constructor__@312
  return new haxe.ValueException(value, null, value);
}

// fun@43
// fun@228 (15 ops)
static function thrown(value: Dynamic): Dynamic {
  // isOfType@224
  var v0 = isOfType(value, haxe.$Exception) /* fun@224 */;
  if (v0) {
    // nullcheck value
    // get_native@308
    var v1 = value.get_native();
    return v1;
  }
  var e = new haxe.ValueException();
  e = new haxe.ValueException(value, null, null);
  // __constructor__@312
  var v2 = e.__skipStack;
  v2++;
  e.__skipStack = v2;
  return e;
}

// fun@44
// fun@310 (13 ops)
static function __constructor__(message: haxe.Exception, previous: String, native: haxe.Exception, _: Dynamic) {
  this.__skipStack = 0;
  this.__exceptionMessage = message;
  this.__previousException = previous;
  if (native != null) {
    this.__nativeStack = exception_stack() /* fun@313 */;
    this.__nativeException = native;
  } else {
    this.__nativeStack = callStack() /* fun@314 */;
    this.__nativeException = this;
  }
}

// fun@50
// fun@214 (32 ops)
static function formatOutput(v: Dynamic, infos: Dynamic): String {
  // string@225
  var str = string(v) /* fun@225 */;
  if (infos == null) {
    return str;
  }
  // nullcheck infos
  // __add__@20
  var v0 = infos.fileName + ":";
  // itos@216
  var v1 = infos.lineNumber;
  // __alloc__@16
  var v2 = v1;
  // __add__@20
  var pstr = v0 + v2;
  if (infos.customParams != null) {
    while (true) {
      // nullcheck infos.customParams
      // get_length@26
      var v3 = infos.customParams.get_length();
      if (v3 > 0) {
        v = infos.customParams.array(0);
        var v4 = 0;
        v4++;
        // string@225
        var v5 = string(v) /* fun@225 */;
        // __add__@20
        var v6 = ", " + v5;
        // __add__@20
        var v7 = str + v6;
        str = v7;
      }
    }
  }
  // __add__@20
  var v8 = pstr + ": ";
  // __add__@20
  var v9 = v8 + str;
  return v9;
}

// fun@51
// fun@215 (3 ops)
static function trace(v: Dynamic, infos: Dynamic) {
  // formatOutput@214
  var str = formatOutput(v, infos) /* fun@214 */;
  // println@289
  println(str) /* fun@289 */;
}

// fun@52
// fun@315 (1 ops)
static function saveStack(exception: Dynamic) {}

// fun@53
// fun@314 (43 ops)
static function callStack(): Array<Dynamic> {
  try {
    // __constructor__@310
    throw new haxe.Exception("", null, "stack");
  } catch (e) {
    var stack = exception_stack() /* fun@313 */;
    var skip = 1;
    while (true) {
      if (stack.length - 1 > 0) {
        var i = 0;
        i++;
        var s = new String();
        // ucs2length@230
        var v0 = ucs2length(stack[i], 0) /* fun@230 */;
        s = new String("NativeStackTrace.callStack", null);
        // indexOf@5
        if (0 > indexOf(s, "NativeStackTrace.callStack", null) /* fun@5 */) {
        } else {
          skip++;
        }
      }
    }
    if (stack.length <= skip) {
      return stack;
    }
    // alloc_array@272
    var v1 = alloc_array(haxe.io.Bytes, stack.length - skip) /* fun@272 */;
    // array_blit@316
    array_blit(v1, 0, stack, skip, stack.length - skip) /* fun@316 */;
    return v1;
  }
  return stack;
}

// fun@54
// fun@312 (4 ops)
static function __constructor__(value: haxe.ValueException, previous: Dynamic, native: haxe.Exception, _: Dynamic) {
  // string@225
  var v0 = string(value) /* fun@225 */;
  // __constructor__@310
  __constructor__(this, v0, previous, native) /* fun@310 */;
  this.value = value;
}

// fun@56
// fun@317 (5 ops)
static function sort(a: hl.types.ArrayDyn, cmp: Function) {
  // nullcheck a
  // get_length@26
  var v0 = a.get_length();
  // rec@318
  rec(a, cmp, 0, v0) /* fun@318 */;
}

// fun@57
// fun@318 (42 ops)
static function rec(a: hl.types.ArrayDyn, cmp: Function, from: Int, to: Int) {
  var middle = from + to >> 1;
  if (12 > to - from) {
    if (to <= from) {
      return;
    }
    while (true) {
      if (to <= from + 1) {
        return;
      }
      var i = from + 1;
      i++;
      var j = i;
      while (true) {
        if (j > from) {
          // nullcheck cmp
          // nullcheck a
          if (0 > cmp(a.array(j), a.array(j - 1))) {
            // swap@319
            swap(a, j - 1, j) /* fun@319 */;
          } else {
            break;
          }
          j--;
        }
      }
    }
    return;
  }
  // rec@318
  rec(a, cmp, from, middle) /* fun@318 */;
  // rec@318
  rec(a, cmp, middle, v0) /* fun@318 */;
  // doMerge@320
  doMerge(a, cmp, from, middle, v0, middle - from, v0 - middle) /* fun@320 */;
}

// fun@58
// fun@320 (44 ops)
static function doMerge(a: hl.types.ArrayDyn, cmp: Function, from: Int, pivot: Int, to: Int, len1: Int, len2: Int) {
  if (len1 == 0) {
    return;
  }
  if (len2 == 0) {
    return;
  }
  if (len1 + len2 == 2) {
    // nullcheck cmp
    // nullcheck a
    if (0 <= cmp(a.array(pivot), a.array(from))) {
      return;
    }
    // swap@319
    swap(a, pivot, from) /* fun@319 */;
    return;
  }
  if (len1 > len2) {
    var len11 = len1 >> 1;
    var first_cut = from + len11;
    // lower@321
    var v0 = lower(a, cmp, pivot, to, first_cut) /* fun@321 */;
    var second_cut = v0;
    var len22 = second_cut - pivot;
  } else {
    len22 = len2 >> 1;
    second_cut = pivot + len22;
    // upper@322
    var v1 = upper(a, cmp, from, pivot, second_cut) /* fun@322 */;
    first_cut = v1;
    len11 = first_cut - from;
  }
  // rotate@323
  rotate(a, cmp, first_cut, pivot, second_cut) /* fun@323 */;
  var new_mid = first_cut + len22;
  // doMerge@320
  doMerge(a, cmp, from, first_cut, new_mid, len11, len22) /* fun@320 */;
  // doMerge@320
  doMerge(a, cmp, new_mid, second_cut, to, len1 - len11, len2 - len22) /* fun@320 */;
}

// fun@59
// fun@323 (39 ops)
static function rotate(a: hl.types.ArrayDyn, cmp: Function, from: Int, mid: Int, to: Int) {
  if (from == mid) {
    return;
  }
  if (mid == to) {
    return;
  }
  // gcd@324
  var n = gcd(to - from, mid - from) /* fun@324 */;
  while (true) {
    v0--;
    if (n == 0) {
      return;
    }
    // nullcheck a
    var val = a.array(from + v0);
    var shift = mid - from;
    var p1 = from + v0;
    var p2 = from + v0 + shift;
    while (true) {
      if (p2 != from + v0) {
        // nullcheck a
        a.allowReinterpret(p1, a.array(p2));
        p1 = p2;
        p2 = if (to - p1 > shift) {
          p1 + shift;
        } else {
          from + shift - to - p2;
        };
      }
    }
    // nullcheck a
    a.allowReinterpret(p1, val);
  }
}

// fun@60
// fun@324 (8 ops)
static function gcd(m: Int, n: Int): Int {
  while (true) {
    if (n == 0) {
      return m;
    }
    var t = m % n;
    m = n;
    n = t;
  }
  return m;
}

// fun@61
// fun@322 (27 ops)
static function upper(a: hl.types.ArrayDyn, cmp: Function, from: Int, to: Int, val: Int): Int {
  var len = to - from;
  while (true) {
    if (len <= 0) {
      return from;
    }
    var half = len >> 1;
    var mid = from + half;
    // nullcheck cmp
    // nullcheck a
    len = if (0 > cmp(a.array(val), a.array(mid))) {
      half;
    } else {
      from = mid + 1;
      len - len - 1;
    };
  }
  return from;
}

// fun@62
// fun@321 (27 ops)
static function lower(a: hl.types.ArrayDyn, cmp: Function, from: Int, to: Int, val: Int): Int {
  var len = to - from;
  while (true) {
    if (len <= 0) {
      return from;
    }
    var half = len >> 1;
    var mid = from + half;
    // nullcheck cmp
    // nullcheck a
    len = if (0 > cmp(a.array(mid), a.array(val))) {
      from = mid + 1;
      len - half - 1;
    } else {
      half;
    };
  }
  return from;
}

// fun@63
// fun@319 (6 ops)
static function swap(a: hl.types.ArrayDyn, i: Int, j: Int) {
  // nullcheck a
  var tmp = a.array(i);
  a.allowReinterpret(i, a.array(j));
  a.allowReinterpret(j, tmp);
}

// fun@64
// fun@325 (4 ops)
static function __constructor__(array: haxe.iterators.ArrayIterator, _: hl.types.ArrayDyn) {
  this.current = 0;
  this.array = array;
}

// fun@67
// fun@328 (4 ops)
static function __constructor__(array: haxe.iterators.ArrayKeyValueIterator, _: hl.types.ArrayDyn) {
  this.current = 0;
  this.array = array;
}

// fun@69
// fun@345 (6 ops)
static function __constructor__(arr: hl.NativeArrayIterator_Dynamic, _: Array<Dynamic>) {
  this.arr = arr;
  this.pos = 0;
  this.length = arr.length;
}

// fun@72
// fun@348 (6 ops)
static function __constructor__(arr: hl.NativeArrayIterator_Int, _: Array<Dynamic>) {
  this.arr = arr;
  this.pos = 0;
  this.length = arr.length;
}

// fun@78
// fun@73 (5 ops)
static function allocI32(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_Int {
  var a = new hl.types.ArrayBytes_Int();
  return a;
}

// fun@79
// fun@74 (5 ops)
static function allocUI16(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_hl_UI16 {
  var a = new hl.types.ArrayBytes_hl_UI16();
  return a;
}

// fun@80
// fun@75 (5 ops)
static function allocF32(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_hl_F32 {
  var a = new hl.types.ArrayBytes_hl_F32();
  return a;
}

// fun@81
// fun@76 (5 ops)
static function allocF64(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_Float {
  var a = new hl.types.ArrayBytes_Float();
  return a;
}

// fun@99
// fun@352 (6 ops)
static function String(_: hl.types.ArrayBytes_Float) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@108
// fun@353 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@133
// fun@360 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@136
// fun@361 (6 ops)
static function String(_: hl.types.ArrayBytes_Int) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@145
// fun@362 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@170
// fun@369 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@173
// fun@145 (6 ops)
static function __constructor__(_: hl.types.ArrayBytes_hl_F32) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@182
// fun@370 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@183
// fun@371 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@208
// fun@378 (4 ops)
static function String(_: Function, _: Single, _: Single): Int {
  return arg0(arg1, arg2);
}

// fun@211
// fun@379 (6 ops)
static function String(_: hl.types.ArrayBytes_hl_UI16) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@220
// fun@380 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@221
// fun@381 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@246
// fun@388 (4 ops)
static function String(_: Function, _: hl.UI16, _: hl.UI16): Int {
  return arg0(arg1, arg2);
}

// fun@249
// fun@389 (5 ops)
static function __constructor__(a: hl.types.ArrayDynIterator, _: hl.types.ArrayBase) {
  // __constructor__@325
  __constructor__(this, null) /* fun@325 */;
  this.a = a;
}

// fun@252
// fun@392 (5 ops)
static function __constructor__(a: hl.types.ArrayDynKeyValueIterator, _: hl.types.ArrayBase) {
  // __constructor__@328
  __constructor__(this, null) /* fun@328 */;
  this.a = a;
}

// fun@253
// fun@351 (8 ops)
static function alloc(a: hl.types.ArrayBase, allowReinterpret: hl.Ref): hl.types.ArrayDyn {
  allowReinterpret = if (allowReinterpret == null) {
    false;
  } else {
    allowReinterpret;
  };
  var arr = new hl.types.ArrayDyn();
  return arr;
}

// fun@254
// fun@393 (6 ops)
static function __constructor__(_: hl.types.ArrayDyn) {
  // String@301
  this.array = new hl.types.ArrayObj();
  this.allowReinterpret = true;
}

// fun@285
// fun@394 (5 ops)
static function __constructor__(arr: hl.types.ArrayObjIterator, _: hl.types.ArrayObj) {
  // __constructor__@325
  __constructor__(this, null) /* fun@325 */;
  this.arr = arr;
}

// fun@288
// fun@397 (5 ops)
static function __constructor__(arr: hl.types.ArrayObjKeyValueIterator, _: hl.types.ArrayObj) {
  // __constructor__@328
  __constructor__(this, null) /* fun@328 */;
  this.arr = arr;
}

// fun@289
// fun@273 (5 ops)
static function String(a: Array<Dynamic>): hl.types.ArrayObj {
  var arr = new hl.types.ArrayObj();
  return arr;
}

// fun@290
// fun@301 (7 ops)
static function String(_: hl.types.ArrayObj) {
  this.length = 0;
  // alloc_array@272
  var v0 = alloc_array(Dynamic, 0) /* fun@272 */;
  this.array = v0;
}

// fun@326
// fun@356 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_Float, _: hl.types.ArrayBytes_Float) {
  // __constructor__@325
  __constructor__(this, null) /* fun@325 */;
  this.a = a;
}

// fun@329
// fun@365 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_Int, _: hl.types.ArrayBytes_Int) {
  // __constructor__@325
  __constructor__(this, null) /* fun@325 */;
  this.a = a;
}

// fun@332
// fun@374 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_hl_F32, _: hl.types.ArrayBytes_hl_F32) {
  // __constructor__@325
  __constructor__(this, null) /* fun@325 */;
  this.a = a;
}

// fun@335
// fun@384 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_hl_UI16, _: hl.types.ArrayBytes_hl_UI16) {
  // __constructor__@325
  __constructor__(this, null) /* fun@325 */;
  this.a = a;
}

// fun@338
// fun@359 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_Float, _: hl.types.ArrayBytes_Float) {
  // __constructor__@328
  __constructor__(this, null) /* fun@328 */;
  this.a = a;
}

// fun@341
// fun@368 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_Int, _: hl.types.ArrayBytes_Int) {
  // __constructor__@328
  __constructor__(this, null) /* fun@328 */;
  this.a = a;
}

// fun@344
// fun@377 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_hl_F32, _: hl.types.ArrayBytes_hl_F32) {
  // __constructor__@328
  __constructor__(this, null) /* fun@328 */;
  this.a = a;
}

// fun@347
// fun@387 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_hl_UI16, _: hl.types.ArrayBytes_hl_UI16) {
  // __constructor__@328
  __constructor__(this, null) /* fun@328 */;
  this.a = a;
}

// fun@350
// fun@398 (228 ops)
static function String() {
  init() /* fun@292 */;
  // initClass@294
  var v0 = initClass($Arithmetic, Arithmetic, "Arithmetic") /* fun@294 */;
  // initClass@294
  var v1 = initClass($Date, Date, "Date") /* fun@294 */;
  // initClass@294
  var v2 = initClass($Std, Std, "Std") /* fun@294 */;
  @G59 = new hl.CoreType();
  // register@297
  register("Float", new hl.CoreType()) /* fun@297 */;
  @G61 = new hl.CoreType();
  // register@297
  register("Int", new hl.CoreType()) /* fun@297 */;
  @G63 = new hl.CoreEnum();
  // register@297
  register("Bool", new hl.CoreEnum()) /* fun@297 */;
  @G65 = new hl.CoreType();
  // register@297
  register("Dynamic", new hl.CoreType()) /* fun@297 */;
  // initClass@294
  var v3 = initClass($String, String, "String") /* fun@294 */;
  // initClass@294
  var v4 = initClass($StringBuf, StringBuf, "StringBuf") /* fun@294 */;
  // initClass@294
  var v5 = initClass($SysError, SysError, "SysError") /* fun@294 */;
  // initClass@294
  var v6 = initClass(hl._Bytes.$Bytes_Impl_, hl._Bytes.Bytes_Impl_, "hl._Bytes.Bytes_Impl_") /* fun@294 */;
  // initClass@294
  var v7 = initClass($Sys, Sys, "Sys") /* fun@294 */;
  // initClass@294
  var v8 = initClass($Type, Type, "Type") /* fun@294 */;
  // initClass@294
  var v9 = initClass(haxe.$Exception, haxe.Exception, "haxe.Exception") /* fun@294 */;
  // initClass@294
  var v10 = initClass(haxe.$Log, haxe.Log, "haxe.Log") /* fun@294 */;
  // initClass@294
  var v11 = initClass(haxe.$NativeStackTrace, haxe.NativeStackTrace, "haxe.NativeStackTrace") /* fun@294 */;
  // initClass@294
  var v12 = initClass(haxe.$ValueException, haxe.ValueException, "haxe.ValueException") /* fun@294 */;
  // initClass@294
  var v13 = initClass(haxe.ds.$ArraySort, haxe.ds.ArraySort, "haxe.ds.ArraySort") /* fun@294 */;
  // initEnum@298
  var v14 = initEnum(haxe.io.$Error, haxe.io.Error) /* fun@298 */;
  @G72 = v14.__evalues__[0];
  @G73 = v14.__evalues__[1];
  @G42 = v14.__evalues__[2];
  // initClass@294
  var v15 = initClass(haxe.iterators.$ArrayIterator, haxe.iterators.ArrayIterator, "haxe.iterators.ArrayIterator") /* fun@294 */;
  // initClass@294
  var v16 = initClass(haxe.iterators.$ArrayKeyValueIterator, haxe.iterators.ArrayKeyValueIterator, "haxe.iterators.ArrayKeyValueIterator") /* fun@294 */;
  // initClass@294
  var v17 = initClass(hl.$BaseType, hl.BaseType, "hl.BaseType") /* fun@294 */;
  // initClass@294
  var v18 = initClass(hl.Class, hl.Class, "Class") /* fun@294 */;
  // initClass@294
  var v19 = initClass(hl.$Enum, hl.Enum, "hl.Enum") /* fun@294 */;
  // initClass@294
  var v20 = initClass(hl._NativeArray.$NativeArray_Impl_, hl._NativeArray.NativeArray_Impl_, "hl._NativeArray.NativeArray_Impl_") /* fun@294 */;
  // initClass@294
  var v21 = initClass(hl.$NativeArrayIterator_Dynamic, hl.NativeArrayIterator_Dynamic, "hl.NativeArrayIterator_Dynamic") /* fun@294 */;
  // initClass@294
  var v22 = initClass(hl.$NativeArrayIterator_Int, hl.NativeArrayIterator_Int, "hl.NativeArrayIterator_Int") /* fun@294 */;
  // initClass@294
  var v23 = initClass(hl._Type.$Type_Impl_, hl._Type.Type_Impl_, "hl._Type.Type_Impl_") /* fun@294 */;
  // initClass@294
  var v24 = initClass(hl.types.$ArrayAccess, hl.types.ArrayAccess, "hl.types.ArrayAccess") /* fun@294 */;
  @G4 = new hl.types.$ArrayBase();
  // register@297
  register("Array", new hl.types.$ArrayBase()) /* fun@297 */;
  // initClass@294
  var v25 = initClass(hl.types.$ArrayBytes_hl_F32, hl.types.ArrayBytes_hl_F32, "hl.types.ArrayBytes_hl_F32") /* fun@294 */;
  // initClass@294
  var v26 = initClass(hl.types.$ArrayDynIterator, hl.types.ArrayDynIterator, "hl.types.ArrayDynIterator") /* fun@294 */;
  // initClass@294
  var v27 = initClass(hl.types.$ArrayDynKeyValueIterator, hl.types.ArrayDynKeyValueIterator, "hl.types.ArrayDynKeyValueIterator") /* fun@294 */;
  @G77 = new hl.types.$ArrayDyn();
  // register@297
  register("hl.types.ArrayDyn", new hl.types.$ArrayDyn()) /* fun@297 */;
  // initClass@294
  var v28 = initClass(hl.types.$ArrayObjIterator, hl.types.ArrayObjIterator, "hl.types.ArrayObjIterator") /* fun@294 */;
  // initClass@294
  var v29 = initClass(hl.types.$ArrayObjKeyValueIterator, hl.types.ArrayObjKeyValueIterator, "hl.types.ArrayObjKeyValueIterator") /* fun@294 */;
  // initClass@294
  var v30 = initClass(hl.types.$BytesIterator_Float, hl.types.BytesIterator_Float, "hl.types.BytesIterator_Float") /* fun@294 */;
  // initClass@294
  var v31 = initClass(hl.types.$BytesIterator_Int, hl.types.BytesIterator_Int, "hl.types.BytesIterator_Int") /* fun@294 */;
  // initClass@294
  var v32 = initClass(hl.types.$BytesIterator_hl_F32, hl.types.BytesIterator_hl_F32, "hl.types.BytesIterator_hl_F32") /* fun@294 */;
  // initClass@294
  var v33 = initClass(hl.types.$BytesIterator_hl_UI16, hl.types.BytesIterator_hl_UI16, "hl.types.BytesIterator_hl_UI16") /* fun@294 */;
  // initClass@294
  var v34 = initClass(hl.types.$BytesKeyValueIterator_Float, hl.types.BytesKeyValueIterator_Float, "hl.types.BytesKeyValueIterator_Float") /* fun@294 */;
  // initClass@294
  var v35 = initClass(hl.types.$BytesKeyValueIterator_Int, hl.types.BytesKeyValueIterator_Int, "hl.types.BytesKeyValueIterator_Int") /* fun@294 */;
  // initClass@294
  var v36 = initClass(hl.types.$BytesKeyValueIterator_hl_F32, hl.types.BytesKeyValueIterator_hl_F32, "hl.types.BytesKeyValueIterator_hl_F32") /* fun@294 */;
  // initClass@294
  var v37 = initClass(hl.types.$BytesKeyValueIterator_hl_UI16, hl.types.BytesKeyValueIterator_hl_UI16, "hl.types.BytesKeyValueIterator_hl_UI16") /* fun@294 */;
  // initClass@294
  var v38 = initClass(hl.types._BytesMap.$BytesMap_Impl_, hl.types._BytesMap.BytesMap_Impl_, "hl.types._BytesMap.BytesMap_Impl_") /* fun@294 */;
  main() /* fun@22 */;
}

