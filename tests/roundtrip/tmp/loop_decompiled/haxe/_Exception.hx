package haxe;

class $Exception extends Class {

  // fun@303 (9 ops)
  dynamic function caught(): haxe.Exception {
    // isOfType@223
    var v0 = isOfType(value, haxe.$Exception) /* fun@223 */;
    if (v0) {
      return value;
    }
    // __constructor__@311
    return new haxe.ValueException(value, null, value);
  }

  // fun@309 (13 ops)
  dynamic function __constructor__(previous: String, native: haxe.Exception, _: Dynamic) {
    this.__skipStack = 0;
    this.__exceptionMessage = message;
    this.__previousException = previous;
    if (native != null) {
      this.__nativeStack = exception_stack() /* fun@312 */;
      this.__nativeException = native;
    } else {
      this.__nativeStack = callStack() /* fun@313 */;
      this.__nativeException = this;
    }
  }

  // fun@227 (15 ops)
  dynamic function thrown(): Dynamic {
    // isOfType@223
    var v0 = isOfType(value, haxe.$Exception) /* fun@223 */;
    if (v0) {
      // nullcheck value
      // get_native@307
      var v1 = value.get_native();
      return v1;
    }
    var e = new haxe.ValueException();
    e = new haxe.ValueException(value, null, null);
    // __constructor__@311
    var v2 = e.__skipStack;
    v2++;
    e.__skipStack = v2;
    return e;
  }
}