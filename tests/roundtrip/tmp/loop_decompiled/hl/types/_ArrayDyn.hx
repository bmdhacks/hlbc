package hl.types;

class $ArrayDyn extends Class {

  // fun@350 (8 ops)
  dynamic function alloc(allowReinterpret: hl.Ref): hl.types.ArrayDyn {
    allowReinterpret = if (allowReinterpret == null) {
      false;
    } else {
      allowReinterpret;
    };
    var arr = new hl.types.ArrayDyn();
    return arr;
  }

  // fun@392 (6 ops)
  dynamic function __constructor__() {
    // String@300
    this.array = new hl.types.ArrayObj();
    this.allowReinterpret = true;
  }
}