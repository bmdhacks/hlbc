package hl.types;

class ArrayObjKeyValueIterator extends ArrayKeyValueIterator {
  var arr: hl.types.ArrayObj;

  // fun@396 (5 ops)
  static function __constructor__(arr: hl.types.ArrayObjKeyValueIterator, _: hl.types.ArrayObj) {
    // __constructor__@327
    __constructor__(this, null) /* fun@327 */;
    this.arr = arr;
  }
}