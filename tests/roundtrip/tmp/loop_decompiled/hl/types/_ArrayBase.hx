package hl.types;

class $ArrayBase extends Class {

  // fun@81 (5 ops)
  dynamic function allocF64(length: Int): hl.types.ArrayBytes_Float {
    var a = new hl.types.ArrayBytes_Float();
    return a;
  }

  // fun@79 (5 ops)
  dynamic function allocUI16(length: Int): hl.types.ArrayBytes_hl_UI16 {
    var a = new hl.types.ArrayBytes_hl_UI16();
    return a;
  }

  // fun@78 (5 ops)
  dynamic function allocI32(length: Int): hl.types.ArrayBytes_Int {
    var a = new hl.types.ArrayBytes_Int();
    return a;
  }

  // fun@80 (5 ops)
  dynamic function allocF32(length: Int): hl.types.ArrayBytes_hl_F32 {
    var a = new hl.types.ArrayBytes_hl_F32();
    return a;
  }
}