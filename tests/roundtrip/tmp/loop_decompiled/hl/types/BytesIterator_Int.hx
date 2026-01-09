package hl.types;

class BytesIterator_Int extends ArrayIterator {
  var a: hl.types.ArrayBytes_Int;

  // fun@364 (5 ops)
  static function __constructor__(a: hl.types.BytesIterator_Int, _: hl.types.ArrayBytes_Int) {
    // __constructor__@324
    __constructor__(this, null) /* fun@324 */;
    this.a = a;
  }

  // fun@362 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@363 (12 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v0 = this.current;
    v0++;
    this.current = v0;
    return this.a.bytes[this.current << 2];
  }
}