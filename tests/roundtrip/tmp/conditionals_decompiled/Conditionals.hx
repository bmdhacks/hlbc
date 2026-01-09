class Conditionals {

  // fun@22 (74 ops)
  static function main() {
    var x = 5;
    if (x > 3) {
      // nullcheck haxe.$Log.trace
      trace("x is greater than 3") /* fun@215 */;
    } else {
      // nullcheck haxe.$Log.trace
      trace("x is not greater than 3") /* fun@215 */;
    }
    var y = 10;
    if (y == 10) {
      // nullcheck haxe.$Log.trace
      trace("y equals 10") /* fun@215 */;
    }
    var z = if (x > 0) {
      "positive";
    } else {
      "non-positive";
    };
    // nullcheck haxe.$Log.trace
    // __add__@20
    var v0 = "z=" + z;
    trace(v0) /* fun@215 */;
  }
}