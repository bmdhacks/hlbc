class $Date extends Class {

  // fun@24 (3 ops)
  dynamic function __constructor__(month: Int, day: Int, hour: Int, min: Int, sec: Int, _: Int) {
    // date_new@22
    var v0 = date_new(year, month, day, hour, min, sec) /* fun@22 */;
    this.t = v0;
  }
}