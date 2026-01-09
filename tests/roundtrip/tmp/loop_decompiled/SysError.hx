class SysError {
  var msg: String;

  // fun@281 (2 ops)
  static function __constructor__(msg: SysError, _: String) {
    this.msg = msg;
  }

  // fun@282 (6 ops)
  function toString(): String {
    // __add__@20
    var v0 = "SysError(" + this.msg;
    // __add__@20
    var v1 = v0 + ")";
    return v1;
  }

  // fun@283 (4 ops)
  function __string(): hl.Bytes {
    // toString@282
    var v0 = this.toString();
    // nullcheck v0
    return v0.bytes;
  }
}