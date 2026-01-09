class Type {

  // fun@293 (13 ops)
  static function initClass(ct: Class<Dynamic>, t: Class<Dynamic>, name: hl.Bytes): hl.Class {
    // alloc_obj@294
    var v0 = alloc_obj(ct) /* fun@294 */;
    var c = v0;
    // type_set_global@295
    var v1 = type_set_global(t, c) /* fun@295 */;
    // nullcheck c
    c.__type__ = t;
    // ucs2length@229
    var v2 = ucs2length(name, 0) /* fun@229 */;
    // register@296
    register(name, c) /* fun@296 */;
    return c;
  }

  // fun@296 (3 ops)
  static function register(b: hl.Bytes, t: hl.BaseType) {
    // hbset@302
    hbset(@G24, b, t) /* fun@302 */;
  }

  // fun@297 (49 ops)
  static function initEnum(et: Class<Dynamic>, t: Class<Dynamic>): hl.Enum {
    // alloc_obj@294
    var v0 = alloc_obj(et) /* fun@294 */;
    var e = v0;
    // nullcheck e
    e.__type__ = t;
    // type_enum_values@298
    var v1 = type_enum_values(t) /* fun@298 */;
    e.__evalues__ = v1;
    // type_name@299
    var v2 = type_name(t) /* fun@299 */;
    if (v2 == null) {
    } else {
      // ucs2length@229
      var v3 = ucs2length(v2, 0) /* fun@229 */;
    }
    // String@300
    // type_enum_fields@301
    var cl = type_enum_fields(t) /* fun@301 */;
    while (cl.length > 0) {
      var i = 0;
      var v4 = 0;
      v4++;
      var name = cl[i];
      // nullcheck e
      // hbset@302
      hbset(e.__emap__, name, i) /* fun@302 */;
      // ucs2length@229
      var v5 = ucs2length(name, 0) /* fun@229 */;
      // nullcheck e.__constructs__
      // push@240
      var v6 = e.__constructs__.push(new String());
    }
    // nullcheck e
    // nullcheck e.__ename__
    // register@296
    register(e.__ename__.bytes, e) /* fun@296 */;
    // type_set_global@295
    var v7 = type_set_global(t, e) /* fun@295 */;
    return e;
  }

  // fun@291 (3 ops)
  static function init() {
    @G24 = hballoc() /* fun@292 */;
  }
}