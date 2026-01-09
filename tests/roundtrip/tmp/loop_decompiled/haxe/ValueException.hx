package haxe;

class ValueException extends Exception {
  var value: Dynamic;

  // fun@311 (4 ops)
  static function __constructor__(value: haxe.ValueException, previous: Dynamic, native: haxe.Exception, _: Dynamic) {
    // string@224
    var v0 = string(value) /* fun@224 */;
    // __constructor__@309
    __constructor__(this, v0, previous, native) /* fun@309 */;
    this.value = value;
  }

  // fun@310 (2 ops)
  function unwrap(): Dynamic {
    return this.value;
  }
}