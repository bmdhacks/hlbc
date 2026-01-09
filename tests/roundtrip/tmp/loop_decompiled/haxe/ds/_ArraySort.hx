package haxe.ds;

class $ArraySort extends Class {

  // fun@318 (6 ops)
  dynamic function swap(i: Int, j: Int) {
    // nullcheck a
    var tmp = a.array(i);
    a.allowReinterpret(i, a.array(j));
    a.allowReinterpret(j, tmp);
  }

  // fun@320 (27 ops)
  dynamic function lower(cmp: Function, from: Int, to: Int, val: Int): Int {
    var len = to - from;
    while (len > 0) {
      var half = len >> 1;
      var mid = from + len >> 1;
      // nullcheck cmp
      // nullcheck a
      len = if (0 > cmp(a.array(from + len >> 1), a.array(val))) {
        from = mid + 1;
        len - half - 1;
      } else {
        half;
      };
    }
    return from;
  }

  // fun@321 (27 ops)
  dynamic function upper(cmp: Function, from: Int, to: Int, val: Int): Int {
    var len = to - from;
    while (len > 0) {
      var half = len >> 1;
      var mid = from + len >> 1;
      // nullcheck cmp
      // nullcheck a
      len = if (0 > cmp(a.array(val), a.array(mid))) {
        half;
      } else {
        from = mid + 1;
        len - half - 1;
      };
    }
    return from;
  }

  // fun@317 (42 ops)
  dynamic function rec(cmp: Function, from: Int, to: Int) {
    var middle = from + to >> 1;
    if (12 > to - from) {
      if (to <= from) {
        return;
      }
      while (to > from + 1) {
        var i = from + 1;
        var v0 = from + 1;
        v0++;
        var j = i;
        while (j > from) {
          // nullcheck cmp
          // nullcheck a
          if (0 > cmp(a.array(j), a.array(j - 1))) {
            // swap@318
            swap(a, j - 1, j) /* fun@318 */;
          } else {
            break;
          }
          j--;
        }
      }
      return;
    }
    // rec@317
    rec(a, cmp, from, middle) /* fun@317 */;
    // rec@317
    rec(a, cmp, middle, to) /* fun@317 */;
    // doMerge@319
    doMerge(a, cmp, from, middle, to, middle - from, to - middle) /* fun@319 */;
  }

  // fun@316 (5 ops)
  dynamic function sort(cmp: Function) {
    // nullcheck a
    // get_length@31
    var v0 = a.get_length();
    // rec@317
    rec(a, cmp, 0, v0) /* fun@317 */;
  }

  // fun@323 (8 ops)
  dynamic function gcd(n: Int): Int {
    while (n != 0) {
      var t = m % n;
      m = n;
      n = t;
    }
    return m;
  }

  // fun@322 (39 ops)
  dynamic function rotate(cmp: Function, from: Int, mid: Int, to: Int) {
    if (from == mid) {
      return;
    }
    if (mid == to) {
      return;
    }
    // gcd@323
    var n = gcd(to - from, mid - from) /* fun@323 */;
    while (n != 0) {
      n--;
      // nullcheck a
      var val = a.array(from + n);
      var shift = mid - from;
      var p1 = from + n;
      var p2 = from + n + shift;
      while (p2 != from + n) {
        // nullcheck a
        a.allowReinterpret(p1, a.array(p2));
        p1 = p2;
        p2 = if (to - p2 > shift) {
          p2 + shift;
        } else {
          from + shift - to - p2;
        };
      }
      // nullcheck a
      a.allowReinterpret(p1, val);
    }
  }

  // fun@319 (44 ops)
  dynamic function doMerge(cmp: Function, from: Int, pivot: Int, to: Int, len1: Int, len2: Int) {
    if (len1 == 0) {
      return;
    }
    if (len2 == 0) {
      return;
    }
    if (len1 + len2 == 2) {
      // nullcheck cmp
      // nullcheck a
      if (0 <= cmp(a.array(pivot), a.array(from))) {
        return;
      }
      // swap@318
      swap(a, pivot, from) /* fun@318 */;
      return;
    }
    if (len1 > len2) {
      var len11 = len1 >> 1;
      var first_cut = from + len1 >> 1;
      // lower@320
      var v0 = lower(a, cmp, pivot, to, from + len1 >> 1) /* fun@320 */;
      var second_cut = v0;
      var len22 = v0 - pivot;
    } else {
      len22 = len2 >> 1;
      second_cut = pivot + len22;
      // upper@321
      var v1 = upper(a, cmp, from, pivot, second_cut) /* fun@321 */;
      first_cut = v1;
      len11 = v1 - from;
    }
    // rotate@322
    rotate(a, cmp, first_cut, pivot, second_cut) /* fun@322 */;
    var new_mid = first_cut + len22;
    // doMerge@319
    doMerge(a, cmp, from, first_cut, new_mid, len11, len22) /* fun@319 */;
    // doMerge@319
    doMerge(a, cmp, new_mid, second_cut, to, len1 - len11, len2 - len22) /* fun@319 */;
  }
}