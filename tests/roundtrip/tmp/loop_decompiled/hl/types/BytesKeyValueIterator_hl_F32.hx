package hl.types;

class BytesKeyValueIterator_hl_F32 extends ArrayKeyValueIterator {
  var a: hl.types.ArrayBytes_hl_F32;

  // fun@376 (5 ops)
  static function __constructor__(a: hl.types.BytesKeyValueIterator_hl_F32, _: hl.types.ArrayBytes_hl_F32) {
    // __constructor__@327
    __constructor__(this, null) /* fun@327 */;
    this.a = a;
  }

  // fun@374 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@375 (15 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v = this.a.bytes[this.current << 2];
    var v0 = this.current;
    v0++;
    this.current = v0;
    return {key: this.current, value: v};
  }
}