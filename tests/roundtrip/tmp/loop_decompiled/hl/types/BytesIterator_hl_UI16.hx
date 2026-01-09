package hl.types;

class BytesIterator_hl_UI16 extends ArrayIterator {
  var a: hl.types.ArrayBytes_hl_UI16;

  // fun@383 (5 ops)
  static function __constructor__(a: hl.types.BytesIterator_hl_UI16, _: hl.types.ArrayBytes_hl_UI16) {
    // __constructor__@324
    __constructor__(this, null) /* fun@324 */;
    this.a = a;
  }

  // fun@381 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@382 (12 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v0 = this.current;
    v0++;
    this.current = v0;
    return this.a.bytes[this.current << 1];
  }
}