package hl;

class NativeArrayIterator_Dynamic {
  var arr: Array<Dynamic>;
  var pos: Int;
  var length: Int;

  // fun@344 (6 ops)
  static function __constructor__(arr: hl.NativeArrayIterator_Dynamic, _: Array<Dynamic>) {
    this.arr = arr;
    this.pos = 0;
    this.length = arr.length;
  }

  // fun@345 (7 ops)
  function hasNext(): Bool {
    if (this.length <= this.pos) {
    }
    return true;
  }

  // fun@346 (7 ops)
  function next(): Dynamic {
    var v0 = this.pos;
    v0++;
    this.pos = v0;
    return this.arr[this.pos];
  }
}