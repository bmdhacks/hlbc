package haxe.iterators;

class ArrayIterator {
  var array: hl.types.ArrayDyn;
  var current: Int;

  // fun@324 (4 ops)
  static function __constructor__(array: haxe.iterators.ArrayIterator, _: hl.types.ArrayDyn) {
    this.current = 0;
    this.array = array;
  }

  // fun@325 (9 ops)
  function hasNext(): Bool {
    // nullcheck this.array
    // get_length@31
    var v0 = this.array.get_length();
    if (v0 <= this.current) {
    }
    return true;
  }

  // fun@326 (8 ops)
  function next(): Dynamic {
    // nullcheck this.array
    var v0 = this.current;
    v0++;
    this.current = v0;
    return this.array.array(this.current);
  }
}