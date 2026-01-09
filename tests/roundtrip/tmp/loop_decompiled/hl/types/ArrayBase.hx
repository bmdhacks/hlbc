package hl.types;

class ArrayBase extends ArrayAccess {
  var length: Int;

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

  // fun@61 (3 ops)
  function pushDyn(_: Dynamic): Int {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@62 (3 ops)
  function popDyn(): Dynamic {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@63 (3 ops)
  function shiftDyn(): Dynamic {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@64 (3 ops)
  function unshiftDyn(_: Dynamic) {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@65 (3 ops)
  function insertDyn(v: Int, _: Dynamic) {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@66 (3 ops)
  function containsDyn(_: Dynamic): Bool {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@67 (3 ops)
  function removeDyn(_: Dynamic): Bool {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@68 (3 ops)
  function sortDyn(_: Function) {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@69 (3 ops)
  function slice(end: Int, _: Null): hl.types.ArrayBase {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@70 (3 ops)
  function splice(len: Int, _: Int): hl.types.ArrayBase {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@71 (3 ops)
  function join(_: String): String {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@72 (3 ops)
  function reverse() {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@73 (3 ops)
  function resize(_: Int) {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@74 (3 ops)
  function toString(): String {
    // thrown@227
    var v0 = "Not implemented";
    throw v0;
  }

  // fun@75 (9 ops)
  function __cast(_: Class<Dynamic>): Dynamic {
    if (t == hl.types.ArrayDyn) {
      // alloc@350
      var v0 = alloc(this, false) /* fun@350 */;
      return v0;
    }
    return null;
  }

  // fun@76 (2 ops)
  function isArrayObj(): Bool {
    return false;
  }

  // fun@77 (4 ops)
  function __string(): hl.Bytes {
    // nullcheck r2
    return this.toString().bytes;
  }
}