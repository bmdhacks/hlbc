class Date {
  var t: Int;

  // fun@24 (3 ops)
  static function __constructor__(year: Date, month: Int, day: Int, hour: Int, min: Int, sec: Int, _: Int) {
    // date_new@22
    var v0 = date_new(year, month, day, hour, min, sec) /* fun@22 */;
    this.t = v0;
  }

  // fun@25 (8 ops)
  function toString(): String {
    var outLen = 0;
    // date_to_string@23
    var bytes = date_to_string(this.t, outLen) /* fun@23 */;
    return new String();
  }

  // fun@26 (4 ops)
  function __string(): hl.Bytes {
    // toString@25
    var v0 = this.toString();
    // nullcheck v0
    return v0.bytes;
  }
}