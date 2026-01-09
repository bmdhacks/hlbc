class $Loop extends Class {

  // fun@27 (56 ops)
  dynamic function main() {
    var sum = 0;
    var i = 0;
    while (5 > i) {
      sum = sum + i;
      i++;
    }
    // nullcheck haxe.$Log.trace
    // itos@221
    var v0 = sum;
    // __alloc__@16
    var v1 = v0;
    // __add__@20
    var v2 = "sum=" + v1;
    trace(v2) /* fun@220 */;
    var count = 0;
    while (3 > count) {
      // nullcheck haxe.$Log.trace
      // itos@221
      var v3 = count;
      // __alloc__@16
      var v4 = v3;
      // __add__@20
      var v5 = "count=" + v4;
      trace(v5) /* fun@220 */;
      count++;
    }
  }
}