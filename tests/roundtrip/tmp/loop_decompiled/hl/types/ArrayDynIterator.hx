package hl.types;

class ArrayDynIterator extends ArrayIterator {
  var a: hl.types.ArrayBase;

  // fun@388 (5 ops)
  static function __constructor__(a: hl.types.ArrayDynIterator, _: hl.types.ArrayBase) {
    // __constructor__@324
    __constructor__(this, null) /* fun@324 */;
    this.a = a;
  }

  // fun@389 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.a
    if (this.a.length <= this.current) {
    }
    return true;
  }

  // fun@390 (8 ops)
  function next(): Dynamic {
    // nullcheck this.a
    var v0 = this.current;
    v0++;
    this.current = v0;
    return this.a.length(this.current);
  }
}