package haxe.iterators;

class ArrayKeyValueIterator {
  var current: Int;
  var array: hl.types.ArrayDyn;

  // fun@327 (4 ops)
  static function __constructor__(array: haxe.iterators.ArrayKeyValueIterator, _: hl.types.ArrayDyn) {
    this.current = 0;
    this.array = array;
  }
}