package hl._Bytes;

class $Bytes_Impl_ extends Class {

  // fun@287 (4 ops)
  dynamic function sub(pos: Int, size: Int): hl.Bytes {
    // alloc_bytes@228
    var b = alloc_bytes(size) /* fun@228 */;
    // bytes_blit@231
    bytes_blit(b, 0, this1, pos, size) /* fun@231 */;
    return b;
  }
}