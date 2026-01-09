package hl.types;

class BytesIterator_Float extends ArrayIterator {
  var a: hl.types.ArrayBytes_Float;

  // fun@355 (5 ops)
  static function __constructor__(a: hl.types.BytesIterator_Float, _: hl.types.ArrayBytes_Float) {
    // __constructor__@324
    __constructor__(this, null) /* fun@324 */;
    this.a = a;
  }

  // fun@353 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@354 (12 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v0 = this.current;
    v0++;
    this.current = v0;
    return this.a.bytes[this.current << 3];
  }
}