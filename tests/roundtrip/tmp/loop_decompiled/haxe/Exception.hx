package haxe;

class Exception {
  var __exceptionMessage: String;
  var __nativeStack: Array<Dynamic>;
  var __skipStack: Int;
  var __nativeException: Dynamic;
  var __previousException: haxe.Exception;

  // fun@303 (9 ops)
  static function caught(value: Dynamic): haxe.Exception {
    // isOfType@223
    var v0 = isOfType(value, haxe.$Exception) /* fun@223 */;
    if (v0) {
      return value;
    }
    // __constructor__@311
    return new haxe.ValueException(value, null, value);
  }

  // fun@309 (13 ops)
  static function __constructor__(message: haxe.Exception, previous: String, native: haxe.Exception, _: Dynamic) {
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
  static function thrown(value: Dynamic): Dynamic {
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

  // fun@304 (2 ops)
  function unwrap(): Dynamic {
    return this.__nativeException;
  }

  // fun@305 (2 ops)
  function toString(): String {
    // get_message@306
    var v0 = this.get_message();
    return v0;
  }

  // fun@306 (2 ops)
  function get_message(): String {
    return this.__exceptionMessage;
  }

  // fun@307 (2 ops)
  function get_native(): Dynamic {
    return this.__nativeException;
  }

  // fun@308 (4 ops)
  function __string(): hl.Bytes {
    // toString@305
    var v0 = this.toString();
    // nullcheck v0
    return v0.bytes;
  }
}