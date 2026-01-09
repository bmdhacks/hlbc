package haxe;

class $NativeStackTrace extends Class {

  // fun@313 (43 ops)
  dynamic function callStack(): Array<Dynamic> {
    try {
      // __constructor__@309
      throw new haxe.Exception("", null, "stack");
    } catch (e) {
      var stack = exception_stack() /* fun@312 */;
      var skip = 1;
      while (exception_stack() /* fun@312 */.length - 1 > 0) {
        var i = 0;
        var v0 = 0;
        v0++;
        var s = new String();
        // ucs2length@229
        var v1 = ucs2length(stack[i], 0) /* fun@229 */;
        s = new String("NativeStackTrace.callStack", null);
        // indexOf@5
        if (0 > indexOf(s, "NativeStackTrace.callStack", null) /* fun@5 */) {
        } else {
          skip++;
        }
      }
      if (stack.length <= skip) {
        return stack;
      }
      // alloc_array@271
      var v2 = alloc_array(haxe.io.Bytes, stack.length - skip) /* fun@271 */;
      // array_blit@315
      array_blit(v2, 0, stack, skip, stack.length - skip) /* fun@315 */;
      return v2;
    }
    return stack;
  }

  // fun@314 (1 ops)
  dynamic function saveStack() {}
}