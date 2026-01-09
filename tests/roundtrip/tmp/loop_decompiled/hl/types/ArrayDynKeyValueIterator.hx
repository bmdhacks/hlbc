package hl.types;

class ArrayDynKeyValueIterator extends ArrayKeyValueIterator {
  var a: hl.types.ArrayBase;

  // fun@391 (5 ops)
  static function __constructor__(a: hl.types.ArrayDynKeyValueIterator, _: hl.types.ArrayBase) {
    // __constructor__@327
    __constructor__(this, null) /* fun@327 */;
    this.a = a;
  }
}