package hl._Bytes;

class Bytes_Impl_ {

  // fun@287 (4 ops)
  static function sub(this1: hl.Bytes, pos: Int, size: Int): hl.Bytes {
    // alloc_bytes@228
    var b = alloc_bytes(size) /* fun@228 */;
    // bytes_blit@231
    bytes_blit(b, 0, this1, pos, size) /* fun@231 */;
    return b;
  }
}