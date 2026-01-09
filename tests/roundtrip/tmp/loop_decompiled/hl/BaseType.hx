package hl;

class BaseType {
  var __type__: Class<Dynamic>;
  var __meta__: Dynamic;
  var __implementedBy__: Array<Dynamic>;

  // fun@14 (31 ops)
  function check(_: Dynamic): Bool {
    var t = typeof(v);
    if (typeid(t) == 15) {
      // get_virtual_value@338
      var v2 = get_virtual_value(v) /* fun@338 */;
      if (v2 != null) {
        t = typeof(v2);
      }
    }
    if (this.__implementedBy__ == null) {
      // type_safe_cast@342
      var v0 = type_safe_cast(t, this.__type__) /* fun@342 */;
      if (v0) {
        return true;
      }
      return false;
    }
    while (this.__implementedBy__.length > 0) {
      var i = this.__implementedBy__[0];
      var v1 = 0;
      v1++;
      // type_safe_cast@342
      v2 = type_safe_cast(t, i) /* fun@342 */;
      if (v2) {
        return true;
      }
    }
    return false;
  }
}