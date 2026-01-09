class $Std extends Class {
  var rnd: Dynamic;
  var toStringDepth: Int;

  // fun@224 (7 ops)
  dynamic function string(): String {
    var len = 0;
    // value_to_string@225
    var bytes = value_to_string(s, len) /* fun@225 */;
    return new String();
  }

  // fun@223 (37 ops)
  dynamic function isOfType(t: Dynamic): Bool {
    t = t;
    if (t == null) {
      return false;
    }
    // nullcheck t
    switch (typeid(t.__type__)) {
      case 3:
        switch (typeid(typeof(v))) {
          case 5:
            v = v;
            if (v != v) {
            }
            return true;
        }
      case 6:
        switch (typeid(typeof(v))) {
          case 1:
            return true;
        }
      case 9:
        if (v == null) {
        }
        return true;
    }
    // check@14
    var v0 = t.check(v);
    return v0;
  }

  // fun@226 (77 ops)
  dynamic function __add__(b: Dynamic): Dynamic {
    var ta = typeof(a);
    var tb = typeof(b);
    if (ta == String) {
      // string@224
      var v0 = string(b) /* fun@224 */;
      // __add__@20
      var v1 = a + v0;
      return v1;
    }
    if (tb == String) {
      // string@224
      var v2 = string(a) /* fun@224 */;
      // __add__@20
      var v3 = v2 + b;
      return v3;
    }
    switch (typeid(ta)) {
      default:
        switch (typeid(tb)) {
          case 0:
          case 1:
            return a;
          case 5:
            return a + b;
            return a + b;
        }
      case 0:
        switch (typeid(tb)) {
          case 0:
          case 1:
            return 0;
            return b;
        }
      case 1:
        a = a;
        switch (typeid(tb)) {
          case 0:
          case 1:
            return a;
          case 5:
            return a + b;
            return a + b;
        }
      case 5:
        a = a;
    }
    // string@224
    var v4 = string(a) /* fun@224 */;
    // __add__@20
    var v5 = "Can't add " + v4;
    // __add__@20
    var v6 = v5 + "(";
    // string@224
    var v7 = string(ta) /* fun@224 */;
    // __add__@20
    var v8 = v6 + v7;
    // __add__@20
    var v9 = v8 + ") and ";
    // string@224
    var v10 = string(b) /* fun@224 */;
    // __add__@20
    var v11 = v9 + v10;
    // __add__@20
    var v12 = v11 + "(";
    // string@224
    var v13 = string(tb) /* fun@224 */;
    // __add__@20
    var v14 = v12 + v13;
    // __add__@20
    var v15 = v14 + ")";
    // thrown@227
    var v16 = v15;
    throw v16;
  }
}