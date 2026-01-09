package hl.types;

class BytesKeyValueIterator_Int extends ArrayKeyValueIterator {
  var a: hl.types.ArrayBytes_Int;

  // fun@367 (5 ops)
  static function __constructor__(a: hl.types.BytesKeyValueIterator_Int, _: hl.types.ArrayBytes_Int) {
    // __constructor__@327
    __constructor__(this, null) /* fun@327 */;
    this.a = a;
  }

  // fun@365 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@366 (15 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v = this.a.bytes[this.current << 2];
    var v0 = this.current;
    v0++;
    this.current = v0;
    return {key: this.current, value: v};
  }
}