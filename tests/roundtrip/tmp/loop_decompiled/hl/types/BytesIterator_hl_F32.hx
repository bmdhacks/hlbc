package hl.types;

class BytesIterator_hl_F32 extends ArrayIterator {
  var a: hl.types.ArrayBytes_hl_F32;

  // fun@373 (5 ops)
  static function __constructor__(a: hl.types.BytesIterator_hl_F32, _: hl.types.ArrayBytes_hl_F32) {
    // __constructor__@324
    __constructor__(this, null) /* fun@324 */;
    this.a = a;
  }

  // fun@371 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@372 (12 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v0 = this.current;
    v0++;
    this.current = v0;
    return this.a.bytes[this.current << 2];
  }
}