package hl;

class NativeArrayIterator_Int {
  var arr: Array<Dynamic>;
  var pos: Int;
  var length: Int;

  // fun@347 (6 ops)
  static function __constructor__(arr: hl.NativeArrayIterator_Int, _: Array<Dynamic>) {
    this.arr = arr;
    this.pos = 0;
    this.length = arr.length;
  }

  // fun@348 (7 ops)
  function hasNext(): Bool {
    if (this.length <= this.pos) {
    }
    return true;
  }

  // fun@349 (7 ops)
  function next(): Int {
    var v0 = this.pos;
    v0++;
    this.pos = v0;
    return this.arr[this.pos];
  }
}