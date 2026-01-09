class Sys {
  static var utf8Path: Bool;

  // fun@288 (9 ops)
  static function println(v: Dynamic) {
    // string@224
    var v0 = string(v) /* fun@224 */;
    // nullcheck v0
    // sys_print@289
    sys_print(v0.bytes) /* fun@289 */;
    // nullcheck r3
    // sys_print@289
    sys_print("
".bytes) /* fun@289 */;
  }
}