package hl.types;

class ArrayObjIterator extends ArrayIterator {
  var arr: hl.types.ArrayObj;

  // fun@393 (5 ops)
  static function __constructor__(arr: hl.types.ArrayObjIterator, _: hl.types.ArrayObj) {
    // __constructor__@324
    __constructor__(this, null) /* fun@324 */;
    this.arr = arr;
  }

  // fun@394 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.arr
    if (this.arr.length <= this.current) {
    }
    return true;
  }

  // fun@395 (9 ops)
  function next(): Dynamic {
    // nullcheck this.arr
    var v0 = this.current;
    v0++;
    this.current = v0;
    return this.arr.array[this.current];
  }
}