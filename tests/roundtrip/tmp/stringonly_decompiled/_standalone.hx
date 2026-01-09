// Standalone functions (not part of any class)

// fun@0
// fun@24 (3 ops)
static function __constructor__(year: Date, month: Int, day: Int, hour: Int, min: Int, sec: Int, _: Int) {
  // date_new@22
  var v0 = date_new(year, month, day, hour, min, sec) /* fun@22 */;
  this.t = v0;
}

// fun@3
// fun@28 (37 ops)
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

// fun@4
// fun@29 (7 ops)
static function string(s: Dynamic): String {
  var len = 0;
  // value_to_string@30
  var bytes = value_to_string(s, len) /* fun@30 */;
  return new String();
}

// fun@5
// fun@31 (77 ops)
static function __add__(a: Dynamic, b: Dynamic): Dynamic {
  var ta = typeof(a);
  var tb = typeof(b);
  if (ta == String) {
    // string@29
    var v0 = string(b) /* fun@29 */;
    // __add__@20
    var v1 = a + v0;
    return v1;
  }
  if (tb == String) {
    // string@29
    var v2 = string(a) /* fun@29 */;
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
  // string@29
  var v4 = string(a) /* fun@29 */;
  // __add__@20
  var v5 = "Can't add " + v4;
  // __add__@20
  var v6 = v5 + "(";
  // string@29
  var v7 = string(ta) /* fun@29 */;
  // __add__@20
  var v8 = v6 + v7;
  // __add__@20
  var v9 = v8 + ") and ";
  // string@29
  var v10 = string(b) /* fun@29 */;
  // __add__@20
  var v11 = v9 + v10;
  // __add__@20
  var v12 = v11 + "(";
  // string@29
  var v13 = string(tb) /* fun@29 */;
  // __add__@20
  var v14 = v12 + v13;
  // __add__@20
  var v15 = v14 + ")";
  // thrown@32
  var v16 = thrown(v15) /* fun@32 */;
  throw v16;
}

// fun@6
// fun@15 (63 ops)
static function fromCharCode(code: Int): String {
  if (0 <= code) {
    if (65536 > code) {
      if (55296 <= code) {
        if (code <= 57343) {
          // itos@33
          var v1 = itos(code, code) /* fun@33 */;
          // __alloc__@16
          var v2 = __alloc__(v1, code) /* fun@16 */;
          // __add__@20
          var v3 = "Invalid unicode char " + v2;
          // thrown@32
          var v4 = thrown(v3) /* fun@32 */;
          throw v4;
        }
      }
      // alloc_bytes@34
      var b = alloc_bytes(4) /* fun@34 */;
      b[0] = v0;
      b[2] = 0;
      return new String();
    }
  }
  if (1114112 > v0) {
    // alloc_bytes@34
    b = alloc_bytes(6) /* fun@34 */;
    code = v0 - 65536;
    b[0] = code >> 10 + 55296;
    b[2] = code && 1023 + 56320;
    b[4] = 0;
    return new String();
  }
  // itos@33
  var v6 = itos(code, code) /* fun@33 */;
  // __alloc__@16
  var v7 = __alloc__(v6, code) /* fun@16 */;
  // thrown@32
  throw thrown(new String()) /* fun@32 */;
}

// fun@7
// fun@16 (4 ops)
static function __alloc__(b: hl.Bytes, length: Int): String {
  var s = new String();
  return s;
}

// fun@8
// fun@17 (8 ops)
static function call_toString(v: Dynamic): hl.Bytes {
  // nullcheck v
  // nullcheck r1
  var s = v["toString"]();
  // nullcheck s
  return s.bytes;
}

// fun@9
// fun@18 (6 ops)
static function fromUCS2(b: hl.Bytes): String {
  var s = new String();
  // ucs2length@35
  var v0 = ucs2length(b, 0) /* fun@35 */;
  return s;
}

// fun@10
// fun@19 (10 ops)
static function fromUTF8(b: hl.Bytes): String {
  var outLen = 0;
  // utf8_to_utf16@36
  var b2 = utf8_to_utf16(b, 0, outLen) /* fun@36 */;
  return new String();
}

// fun@11
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
  // alloc_bytes@34
  var bytes = alloc_bytes(tot + 2) /* fun@34 */;
  // bytes_blit@37
  bytes_blit(bytes, 0, a.bytes, 0, asize) /* fun@37 */;
  // bytes_blit@37
  bytes_blit(bytes, asize, b.bytes, 0, bsize) /* fun@37 */;
  bytes[tot] = 0;
  return new String();
}

// fun@12
// fun@21 (6 ops)
static function __constructor__(string: String, _: String) {
  // nullcheck string
  this.bytes = string.bytes;
  this.length = string.length;
}

// fun@27
// fun@243 (8 ops)
static function __constructor__(_: StringBuf) {
  this.pos = 0;
  this.size = 8;
  // alloc_bytes@34
  var v0 = alloc_bytes(this.size) /* fun@34 */;
  this.b = v0;
}

// fun@32
// fun@248 (31 ops)
static function main() {
  var greeting = "Hello";
  var name = "World";
  // nullcheck haxe.$Log.trace
  trace(greeting) /* fun@280 */;
  // nullcheck haxe.$Log.trace
  trace(name) /* fun@280 */;
}

// fun@33
// fun@281 (2 ops)
static function __constructor__(msg: SysError, _: String) {
  this.msg = msg;
}

// fun@36
// fun@287 (4 ops)
static function sub(this1: hl.Bytes, pos: Int, size: Int): hl.Bytes {
  // alloc_bytes@34
  var b = alloc_bytes(size) /* fun@34 */;
  // bytes_blit@37
  bytes_blit(b, 0, this1, pos, size) /* fun@37 */;
  return b;
}

// fun@37
// fun@288 (9 ops)
static function println(v: Dynamic) {
  // string@29
  var v0 = string(v) /* fun@29 */;
  // nullcheck v0
  // sys_print@289
  sys_print(v0.bytes) /* fun@289 */;
  // nullcheck r3
  // sys_print@289
  sys_print("
".bytes) /* fun@289 */;
}

// fun@38
// fun@291 (3 ops)
static function init() {
  @G24 = hballoc() /* fun@292 */;
}

// fun@39
// fun@293 (13 ops)
static function initClass(ct: Class<Dynamic>, t: Class<Dynamic>, name: hl.Bytes): hl.Class {
  // alloc_obj@294
  var v0 = alloc_obj(ct) /* fun@294 */;
  var c = v0;
  // type_set_global@295
  var v1 = type_set_global(t, c) /* fun@295 */;
  // nullcheck c
  c.__type__ = t;
  // ucs2length@35
  var v2 = ucs2length(name, 0) /* fun@35 */;
  // register@296
  register(name, c) /* fun@296 */;
  return c;
}

// fun@40
// fun@297 (49 ops)
static function initEnum(et: Class<Dynamic>, t: Class<Dynamic>): hl.Enum {
  // alloc_obj@294
  var v0 = alloc_obj(et) /* fun@294 */;
  var e = v0;
  // nullcheck e
  e.__type__ = t;
  // type_enum_values@298
  var v1 = type_enum_values(t) /* fun@298 */;
  e.__evalues__ = v1;
  // type_name@299
  var v2 = type_name(t) /* fun@299 */;
  if (v2 == null) {
  } else {
    // ucs2length@35
    var v3 = ucs2length(v2, 0) /* fun@35 */;
  }
  // String@300
  // type_enum_fields@301
  var cl = type_enum_fields(t) /* fun@301 */;
  while (true) {
    if (cl.length > 0) {
      var i = 0;
      i++;
      var name = cl[i];
      // nullcheck e
      // hbset@302
      hbset(e.__emap__, name, i) /* fun@302 */;
      // ucs2length@35
      var v4 = ucs2length(name, 0) /* fun@35 */;
      // nullcheck e.__constructs__
      // push@70
      var v5 = e.__constructs__.push(new String());
    }
  }
  // nullcheck e
  // nullcheck e.__ename__
  // register@296
  register(e.__ename__.bytes, e) /* fun@296 */;
  // type_set_global@295
  var v6 = type_set_global(t, e) /* fun@295 */;
  return e;
}

// fun@41
// fun@296 (3 ops)
static function register(b: hl.Bytes, t: hl.BaseType) {
  // hbset@302
  hbset(@G24, b, t) /* fun@302 */;
}

// fun@42
// fun@303 (9 ops)
static function caught(value: Dynamic): haxe.Exception {
  // isOfType@28
  var v0 = isOfType(value, haxe.$Exception) /* fun@28 */;
  if (v0) {
    return value;
  }
  // __constructor__@311
  return new haxe.ValueException(value, null, value);
}

// fun@43
// fun@32 (15 ops)
static function thrown(value: Dynamic): Dynamic {
  // isOfType@28
  var v0 = isOfType(value, haxe.$Exception) /* fun@28 */;
  if (v0) {
    // nullcheck value
    // get_native@307
    var v1 = value.get_native();
    return v1;
  }
  var e = new haxe.ValueException();
  e = new haxe.ValueException(value, null, null);
  // __constructor__@311
  var v2 = e.__skipStack;
  v2++;
  e.__skipStack = v2;
  return e;
}

// fun@44
// fun@309 (13 ops)
static function __constructor__(message: haxe.Exception, previous: String, native: haxe.Exception, _: Dynamic) {
  this.__skipStack = 0;
  this.__exceptionMessage = message;
  this.__previousException = previous;
  if (native != null) {
    this.__nativeStack = exception_stack() /* fun@312 */;
    this.__nativeException = native;
  } else {
    this.__nativeStack = callStack() /* fun@313 */;
    this.__nativeException = this;
  }
}

// fun@50
// fun@279 (32 ops)
static function formatOutput(v: Dynamic, infos: Dynamic): String {
  // string@29
  var str = string(v) /* fun@29 */;
  if (infos == null) {
    return str;
  }
  // nullcheck infos
  // __add__@20
  var v0 = infos.fileName + ":";
  // itos@33
  var v1 = itos(infos.lineNumber, infos.lineNumber) /* fun@33 */;
  // __alloc__@16
  var v2 = __alloc__(v1, infos.lineNumber) /* fun@16 */;
  // __add__@20
  var pstr = v0 + v2;
  if (infos.customParams != null) {
    while (true) {
      // nullcheck infos.customParams
      // get_length@249
      var v3 = infos.customParams.get_length();
      if (v3 > 0) {
        v = infos.customParams.array(0);
        var v4 = 0;
        v4++;
        // string@29
        var v5 = string(v) /* fun@29 */;
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
// fun@280 (3 ops)
static function trace(v: Dynamic, infos: Dynamic) {
  // formatOutput@279
  var str = formatOutput(v, infos) /* fun@279 */;
  // println@288
  println(str) /* fun@288 */;
}

// fun@52
// fun@314 (1 ops)
static function saveStack(exception: Dynamic) {}

// fun@53
// fun@313 (43 ops)
static function callStack(): Array<Dynamic> {
  try {
    // __constructor__@309
    throw new haxe.Exception("", null, "stack");
  } catch (e) {
    var stack = exception_stack() /* fun@312 */;
    var skip = 1;
    while (true) {
      if (stack.length - 1 > 0) {
        var i = 0;
        i++;
        var s = new String();
        // ucs2length@35
        var v0 = ucs2length(stack[i], 0) /* fun@35 */;
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
    // alloc_array@238
    var v1 = alloc_array(haxe.io.Bytes, stack.length - skip) /* fun@238 */;
    // array_blit@315
    array_blit(v1, 0, stack, skip, stack.length - skip) /* fun@315 */;
    return v1;
  }
  return stack;
}

// fun@54
// fun@311 (4 ops)
static function __constructor__(value: haxe.ValueException, previous: Dynamic, native: haxe.Exception, _: Dynamic) {
  // string@29
  var v0 = string(value) /* fun@29 */;
  // __constructor__@309
  __constructor__(this, v0, previous, native) /* fun@309 */;
  this.value = value;
}

// fun@56
// fun@316 (5 ops)
static function sort(a: hl.types.ArrayDyn, cmp: Function) {
  // nullcheck a
  // get_length@249
  var v0 = a.get_length();
  // rec@317
  rec(a, cmp, 0, v0) /* fun@317 */;
}

// fun@57
// fun@317 (42 ops)
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
            // swap@318
            swap(a, j - 1, j) /* fun@318 */;
          } else {
            break;
          }
          j--;
        }
      }
    }
    return;
  }
  // rec@317
  rec(a, cmp, from, middle) /* fun@317 */;
  // rec@317
  rec(a, cmp, middle, v0) /* fun@317 */;
  // doMerge@319
  doMerge(a, cmp, from, middle, v0, middle - from, v0 - middle) /* fun@319 */;
}

// fun@58
// fun@319 (44 ops)
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
    // swap@318
    swap(a, pivot, from) /* fun@318 */;
    return;
  }
  if (len1 > len2) {
    var len11 = len1 >> 1;
    var first_cut = from + len11;
    // lower@320
    var v0 = lower(a, cmp, pivot, to, first_cut) /* fun@320 */;
    var second_cut = v0;
    var len22 = second_cut - pivot;
  } else {
    len22 = len2 >> 1;
    second_cut = pivot + len22;
    // upper@321
    var v1 = upper(a, cmp, from, pivot, second_cut) /* fun@321 */;
    first_cut = v1;
    len11 = first_cut - from;
  }
  // rotate@322
  rotate(a, cmp, first_cut, pivot, second_cut) /* fun@322 */;
  var new_mid = first_cut + len22;
  // doMerge@319
  doMerge(a, cmp, from, first_cut, new_mid, len11, len22) /* fun@319 */;
  // doMerge@319
  doMerge(a, cmp, new_mid, second_cut, to, len1 - len11, len2 - len22) /* fun@319 */;
}

// fun@59
// fun@322 (39 ops)
static function rotate(a: hl.types.ArrayDyn, cmp: Function, from: Int, mid: Int, to: Int) {
  if (from == mid) {
    return;
  }
  if (mid == to) {
    return;
  }
  // gcd@323
  var n = gcd(to - from, mid - from) /* fun@323 */;
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
// fun@323 (8 ops)
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
// fun@321 (27 ops)
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
// fun@320 (27 ops)
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
// fun@318 (6 ops)
static function swap(a: hl.types.ArrayDyn, i: Int, j: Int) {
  // nullcheck a
  var tmp = a.array(i);
  a.allowReinterpret(i, a.array(j));
  a.allowReinterpret(j, tmp);
}

// fun@64
// fun@324 (4 ops)
static function __constructor__(array: haxe.iterators.ArrayIterator, _: hl.types.ArrayDyn) {
  this.current = 0;
  this.array = array;
}

// fun@67
// fun@327 (4 ops)
static function __constructor__(array: haxe.iterators.ArrayKeyValueIterator, _: hl.types.ArrayDyn) {
  this.current = 0;
  this.array = array;
}

// fun@69
// fun@344 (6 ops)
static function __constructor__(arr: hl.NativeArrayIterator_Dynamic, _: Array<Dynamic>) {
  this.arr = arr;
  this.pos = 0;
  this.length = arr.length;
}

// fun@72
// fun@347 (6 ops)
static function __constructor__(arr: hl.NativeArrayIterator_Int, _: Array<Dynamic>) {
  this.arr = arr;
  this.pos = 0;
  this.length = arr.length;
}

// fun@78
// fun@62 (5 ops)
static function allocI32(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_Int {
  var a = new hl.types.ArrayBytes_Int();
  return a;
}

// fun@79
// fun@63 (5 ops)
static function allocUI16(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_hl_UI16 {
  var a = new hl.types.ArrayBytes_hl_UI16();
  return a;
}

// fun@80
// fun@64 (5 ops)
static function allocF32(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_hl_F32 {
  var a = new hl.types.ArrayBytes_hl_F32();
  return a;
}

// fun@81
// fun@65 (5 ops)
static function allocF64(bytes: hl.Bytes, length: Int): hl.types.ArrayBytes_Float {
  var a = new hl.types.ArrayBytes_Float();
  return a;
}

// fun@99
// fun@351 (6 ops)
static function String(_: hl.types.ArrayBytes_Float) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@108
// fun@352 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@133
// fun@359 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@136
// fun@360 (6 ops)
static function String(_: hl.types.ArrayBytes_Int) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@145
// fun@361 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@170
// fun@368 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@173
// fun@169 (6 ops)
static function __constructor__(_: hl.types.ArrayBytes_hl_F32) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@182
// fun@369 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@183
// fun@370 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@208
// fun@377 (4 ops)
static function String(_: Function, _: Single, _: Single): Int {
  return arg0(arg1, arg2);
}

// fun@211
// fun@378 (6 ops)
static function String(_: hl.types.ArrayBytes_hl_UI16) {
  this.length = 0;
  this.size = 0;
  this.bytes = null;
}

// fun@220
// fun@379 (4 ops)
static function String(_: Function, _: Int, _: Int): Int {
  return arg0(arg1, arg2);
}

// fun@221
// fun@380 (4 ops)
static function String(_: Function, _: Float, _: Float): Int {
  return arg0(arg1, arg2);
}

// fun@246
// fun@387 (4 ops)
static function String(_: Function, _: hl.UI16, _: hl.UI16): Int {
  return arg0(arg1, arg2);
}

// fun@249
// fun@388 (5 ops)
static function __constructor__(a: hl.types.ArrayDynIterator, _: hl.types.ArrayBase) {
  // __constructor__@324
  __constructor__(this, null) /* fun@324 */;
  this.a = a;
}

// fun@252
// fun@391 (5 ops)
static function __constructor__(a: hl.types.ArrayDynKeyValueIterator, _: hl.types.ArrayBase) {
  // __constructor__@327
  __constructor__(this, null) /* fun@327 */;
  this.a = a;
}

// fun@253
// fun@350 (8 ops)
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
// fun@392 (6 ops)
static function __constructor__(_: hl.types.ArrayDyn) {
  // String@300
  this.array = new hl.types.ArrayObj();
  this.allowReinterpret = true;
}

// fun@285
// fun@393 (5 ops)
static function __constructor__(arr: hl.types.ArrayObjIterator, _: hl.types.ArrayObj) {
  // __constructor__@324
  __constructor__(this, null) /* fun@324 */;
  this.arr = arr;
}

// fun@288
// fun@396 (5 ops)
static function __constructor__(arr: hl.types.ArrayObjKeyValueIterator, _: hl.types.ArrayObj) {
  // __constructor__@327
  __constructor__(this, null) /* fun@327 */;
  this.arr = arr;
}

// fun@289
// fun@239 (5 ops)
static function String(a: Array<Dynamic>): hl.types.ArrayObj {
  var arr = new hl.types.ArrayObj();
  return arr;
}

// fun@290
// fun@300 (7 ops)
static function String(_: hl.types.ArrayObj) {
  this.length = 0;
  // alloc_array@238
  var v0 = alloc_array(Dynamic, 0) /* fun@238 */;
  this.array = v0;
}

// fun@326
// fun@355 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_Float, _: hl.types.ArrayBytes_Float) {
  // __constructor__@324
  __constructor__(this, null) /* fun@324 */;
  this.a = a;
}

// fun@329
// fun@364 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_Int, _: hl.types.ArrayBytes_Int) {
  // __constructor__@324
  __constructor__(this, null) /* fun@324 */;
  this.a = a;
}

// fun@332
// fun@373 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_hl_F32, _: hl.types.ArrayBytes_hl_F32) {
  // __constructor__@324
  __constructor__(this, null) /* fun@324 */;
  this.a = a;
}

// fun@335
// fun@383 (5 ops)
static function __constructor__(a: hl.types.BytesIterator_hl_UI16, _: hl.types.ArrayBytes_hl_UI16) {
  // __constructor__@324
  __constructor__(this, null) /* fun@324 */;
  this.a = a;
}

// fun@338
// fun@358 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_Float, _: hl.types.ArrayBytes_Float) {
  // __constructor__@327
  __constructor__(this, null) /* fun@327 */;
  this.a = a;
}

// fun@341
// fun@367 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_Int, _: hl.types.ArrayBytes_Int) {
  // __constructor__@327
  __constructor__(this, null) /* fun@327 */;
  this.a = a;
}

// fun@344
// fun@376 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_hl_F32, _: hl.types.ArrayBytes_hl_F32) {
  // __constructor__@327
  __constructor__(this, null) /* fun@327 */;
  this.a = a;
}

// fun@347
// fun@386 (5 ops)
static function __constructor__(a: hl.types.BytesKeyValueIterator_hl_UI16, _: hl.types.ArrayBytes_hl_UI16) {
  // __constructor__@327
  __constructor__(this, null) /* fun@327 */;
  this.a = a;
}

// fun@350
// fun@397 (228 ops)
static function String() {
  init() /* fun@291 */;
  // initClass@293
  var v0 = initClass($Date, Date, "Date") /* fun@293 */;
  // initClass@293
  var v1 = initClass($Std, Std, "Std") /* fun@293 */;
  @G55 = new hl.CoreType();
  // register@296
  register("Float", new hl.CoreType()) /* fun@296 */;
  @G57 = new hl.CoreType();
  // register@296
  register("Int", new hl.CoreType()) /* fun@296 */;
  @G59 = new hl.CoreEnum();
  // register@296
  register("Bool", new hl.CoreEnum()) /* fun@296 */;
  @G61 = new hl.CoreType();
  // register@296
  register("Dynamic", new hl.CoreType()) /* fun@296 */;
  // initClass@293
  var v2 = initClass($String, String, "String") /* fun@293 */;
  // initClass@293
  var v3 = initClass($StringBuf, StringBuf, "StringBuf") /* fun@293 */;
  // initClass@293
  var v4 = initClass($StringOnly, StringOnly, "StringOnly") /* fun@293 */;
  // initClass@293
  var v5 = initClass($SysError, SysError, "SysError") /* fun@293 */;
  // initClass@293
  var v6 = initClass(hl._Bytes.$Bytes_Impl_, hl._Bytes.Bytes_Impl_, "hl._Bytes.Bytes_Impl_") /* fun@293 */;
  // initClass@293
  var v7 = initClass($Sys, Sys, "Sys") /* fun@293 */;
  // initClass@293
  var v8 = initClass($Type, Type, "Type") /* fun@293 */;
  // initClass@293
  var v9 = initClass(haxe.$Exception, haxe.Exception, "haxe.Exception") /* fun@293 */;
  // initClass@293
  var v10 = initClass(haxe.$Log, haxe.Log, "haxe.Log") /* fun@293 */;
  // initClass@293
  var v11 = initClass(haxe.$NativeStackTrace, haxe.NativeStackTrace, "haxe.NativeStackTrace") /* fun@293 */;
  // initClass@293
  var v12 = initClass(haxe.$ValueException, haxe.ValueException, "haxe.ValueException") /* fun@293 */;
  // initClass@293
  var v13 = initClass(haxe.ds.$ArraySort, haxe.ds.ArraySort, "haxe.ds.ArraySort") /* fun@293 */;
  // initEnum@297
  var v14 = initEnum(haxe.io.$Error, haxe.io.Error) /* fun@297 */;
  @G69 = v14.__evalues__[0];
  @G70 = v14.__evalues__[1];
  @G39 = v14.__evalues__[2];
  // initClass@293
  var v15 = initClass(haxe.iterators.$ArrayIterator, haxe.iterators.ArrayIterator, "haxe.iterators.ArrayIterator") /* fun@293 */;
  // initClass@293
  var v16 = initClass(haxe.iterators.$ArrayKeyValueIterator, haxe.iterators.ArrayKeyValueIterator, "haxe.iterators.ArrayKeyValueIterator") /* fun@293 */;
  // initClass@293
  var v17 = initClass(hl.$BaseType, hl.BaseType, "hl.BaseType") /* fun@293 */;
  // initClass@293
  var v18 = initClass(hl.Class, hl.Class, "Class") /* fun@293 */;
  // initClass@293
  var v19 = initClass(hl.$Enum, hl.Enum, "hl.Enum") /* fun@293 */;
  // initClass@293
  var v20 = initClass(hl._NativeArray.$NativeArray_Impl_, hl._NativeArray.NativeArray_Impl_, "hl._NativeArray.NativeArray_Impl_") /* fun@293 */;
  // initClass@293
  var v21 = initClass(hl.$NativeArrayIterator_Dynamic, hl.NativeArrayIterator_Dynamic, "hl.NativeArrayIterator_Dynamic") /* fun@293 */;
  // initClass@293
  var v22 = initClass(hl.$NativeArrayIterator_Int, hl.NativeArrayIterator_Int, "hl.NativeArrayIterator_Int") /* fun@293 */;
  // initClass@293
  var v23 = initClass(hl._Type.$Type_Impl_, hl._Type.Type_Impl_, "hl._Type.Type_Impl_") /* fun@293 */;
  // initClass@293
  var v24 = initClass(hl.types.$ArrayAccess, hl.types.ArrayAccess, "hl.types.ArrayAccess") /* fun@293 */;
  @G12 = new hl.types.$ArrayBase();
  // register@296
  register("Array", new hl.types.$ArrayBase()) /* fun@296 */;
  // initClass@293
  var v25 = initClass(hl.types.$ArrayBytes_hl_F32, hl.types.ArrayBytes_hl_F32, "hl.types.ArrayBytes_hl_F32") /* fun@293 */;
  // initClass@293
  var v26 = initClass(hl.types.$ArrayDynIterator, hl.types.ArrayDynIterator, "hl.types.ArrayDynIterator") /* fun@293 */;
  // initClass@293
  var v27 = initClass(hl.types.$ArrayDynKeyValueIterator, hl.types.ArrayDynKeyValueIterator, "hl.types.ArrayDynKeyValueIterator") /* fun@293 */;
  @G74 = new hl.types.$ArrayDyn();
  // register@296
  register("hl.types.ArrayDyn", new hl.types.$ArrayDyn()) /* fun@296 */;
  // initClass@293
  var v28 = initClass(hl.types.$ArrayObjIterator, hl.types.ArrayObjIterator, "hl.types.ArrayObjIterator") /* fun@293 */;
  // initClass@293
  var v29 = initClass(hl.types.$ArrayObjKeyValueIterator, hl.types.ArrayObjKeyValueIterator, "hl.types.ArrayObjKeyValueIterator") /* fun@293 */;
  // initClass@293
  var v30 = initClass(hl.types.$BytesIterator_Float, hl.types.BytesIterator_Float, "hl.types.BytesIterator_Float") /* fun@293 */;
  // initClass@293
  var v31 = initClass(hl.types.$BytesIterator_Int, hl.types.BytesIterator_Int, "hl.types.BytesIterator_Int") /* fun@293 */;
  // initClass@293
  var v32 = initClass(hl.types.$BytesIterator_hl_F32, hl.types.BytesIterator_hl_F32, "hl.types.BytesIterator_hl_F32") /* fun@293 */;
  // initClass@293
  var v33 = initClass(hl.types.$BytesIterator_hl_UI16, hl.types.BytesIterator_hl_UI16, "hl.types.BytesIterator_hl_UI16") /* fun@293 */;
  // initClass@293
  var v34 = initClass(hl.types.$BytesKeyValueIterator_Float, hl.types.BytesKeyValueIterator_Float, "hl.types.BytesKeyValueIterator_Float") /* fun@293 */;
  // initClass@293
  var v35 = initClass(hl.types.$BytesKeyValueIterator_Int, hl.types.BytesKeyValueIterator_Int, "hl.types.BytesKeyValueIterator_Int") /* fun@293 */;
  // initClass@293
  var v36 = initClass(hl.types.$BytesKeyValueIterator_hl_F32, hl.types.BytesKeyValueIterator_hl_F32, "hl.types.BytesKeyValueIterator_hl_F32") /* fun@293 */;
  // initClass@293
  var v37 = initClass(hl.types.$BytesKeyValueIterator_hl_UI16, hl.types.BytesKeyValueIterator_hl_UI16, "hl.types.BytesKeyValueIterator_hl_UI16") /* fun@293 */;
  // initClass@293
  var v38 = initClass(hl.types._BytesMap.$BytesMap_Impl_, hl.types._BytesMap.BytesMap_Impl_, "hl.types._BytesMap.BytesMap_Impl_") /* fun@293 */;
  main() /* fun@248 */;
}

