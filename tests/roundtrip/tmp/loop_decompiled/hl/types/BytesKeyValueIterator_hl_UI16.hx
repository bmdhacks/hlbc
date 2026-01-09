package hl.types;

class BytesKeyValueIterator_hl_UI16 extends ArrayKeyValueIterator {
  var a: hl.types.ArrayBytes_hl_UI16;

  // fun@386 (5 ops)
  static function __constructor__(a: hl.types.BytesKeyValueIterator_hl_UI16, _: hl.types.ArrayBytes_hl_UI16) {
    // __constructor__@327
    __constructor__(this, null) /* fun@327 */;
    this.a = a;
  }

  // fun@384 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@385 (16 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v = this.a.bytes[this.current << 1];
    var v0 = this.current;
    v0++;
    this.current = v0;
    return {key: this.current, value: v};
  }
}