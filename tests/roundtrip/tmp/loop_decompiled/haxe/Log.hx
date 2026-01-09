package haxe;

class Log {

  // fun@219 (32 ops)
  static function formatOutput(v: Dynamic, infos: Dynamic): String {
    // string@224
    var str = string(v) /* fun@224 */;
    if (infos == null) {
      return str;
    }
    // nullcheck infos
    // __add__@20
    var v0 = infos.fileName + ":";
    // itos@221
    var v1 = infos.lineNumber;
    // __alloc__@16
    var v2 = v1;
    // __add__@20
    var pstr = v0 + v2;
    if (infos.customParams != null) {
      while (v3 > 0) {
        // nullcheck infos.customParams
        // get_length@31
        var v3 = infos.customParams.get_length();
        v = infos.customParams.array(0);
        var v4 = 0;
        v4++;
        // string@224
        var v5 = string(v) /* fun@224 */;
        // __add__@20
        var v6 = ", " + v5;
        // __add__@20
        var v7 = str + v6;
        str = v7;
      }
    }
    // __add__@20
    var v8 = pstr + ": ";
    // __add__@20
    var v9 = v8 + str;
    return v9;
  }

  // fun@220 (3 ops)
  static function trace(v: Dynamic, infos: Dynamic) {
    // formatOutput@219
    var str = formatOutput(v, infos) /* fun@219 */;
    // println@288
    println(str) /* fun@288 */;
  }
}