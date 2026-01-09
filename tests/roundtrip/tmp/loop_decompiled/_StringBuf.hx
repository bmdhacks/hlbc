class $StringBuf extends Class {

  // fun@276 (8 ops)
  dynamic function __constructor__() {
    this.pos = 0;
    this.size = 8;
    // alloc_bytes@228
    var v0 = alloc_bytes(this.size) /* fun@228 */;
    this.b = v0;
  }
}