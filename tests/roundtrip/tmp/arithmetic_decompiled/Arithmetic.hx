class Arithmetic {

  // fun@22 (109 ops)
  static function main() {
    var a = 10;
    var b = 3;
    var sum = a + b;
    var diff = a - b;
    var prod = a * b;
    var quot = a / b;
    var mod = a % b;
    // nullcheck haxe.$Log.trace
    // itos@216
    var v1 = sum;
    // __alloc__@16
    var v2 = v1;
    // __add__@20
    var v3 = "sum=" + v2;
    trace(v3) /* fun@215 */;
    // nullcheck haxe.$Log.trace
    // itos@216
    var v5 = diff;
    // __alloc__@16
    var v6 = v5;
    // __add__@20
    var v7 = "diff=" + v6;
    trace(v7) /* fun@215 */;
    // nullcheck haxe.$Log.trace
    // itos@216
    var v9 = prod;
    // __alloc__@16
    var v10 = v9;
    // __add__@20
    var v11 = "prod=" + v10;
    trace(v11) /* fun@215 */;
    // nullcheck haxe.$Log.trace
    // ftos@217
    var v12 = quot;
    // __alloc__@16
    var v13 = v12;
    // __add__@20
    var v14 = "quot=" + v13;
    trace(v14) /* fun@215 */;
    // nullcheck haxe.$Log.trace
    // itos@216
    var v16 = mod;
    // __alloc__@16
    var v17 = v16;
    // __add__@20
    var v18 = "mod=" + v17;
    trace(v18) /* fun@215 */;
  }
}