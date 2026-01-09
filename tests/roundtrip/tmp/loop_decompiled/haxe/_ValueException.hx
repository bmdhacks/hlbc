package haxe;

class $ValueException extends Class {

  // fun@311 (4 ops)
  dynamic function __constructor__(previous: Dynamic, native: haxe.Exception, _: Dynamic) {
    // string@224
    var v0 = string(value) /* fun@224 */;
    // __constructor__@309
    __constructor__(this, v0, previous, native) /* fun@309 */;
    this.value = value;
  }
}