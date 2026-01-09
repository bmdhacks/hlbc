package hl.types;

class BytesKeyValueIterator_Float extends ArrayKeyValueIterator {
  var a: hl.types.ArrayBytes_Float;

  // fun@358 (5 ops)
  static function __constructor__(a: hl.types.BytesKeyValueIterator_Float, _: hl.types.ArrayBytes_Float) {
    // __constructor__@327
    __constructor__(this, null) /* fun@327 */;
    this.a = a;
  }

  // fun@356 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@357 (15 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v = this.a.bytes[this.current << 3];
    var v0 = this.current;
    v0++;
    this.current = v0;
    return {key: this.current, value: v};
  }
}