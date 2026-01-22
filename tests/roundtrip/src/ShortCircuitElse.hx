class ShortCircuitElse {
    // This pattern: if (a && b) { } else { body }
    // compiles to nested jumps where both paths can reach the body,
    // which can confuse the if-pattern detector.
    //
    // Key: both branches must fall through to common exit (no early return in else).
    // This mirrors hxd.BitmapData constructor pattern.
    public var data:Int;

    public function new(width:Int, height:Int) {
        if (width == -101 && height == -102) {
            // no alloc - empty then branch
        } else {
            // Body with multiple statements, no early return
            data = width * height * 4;
            trace("Allocated " + data + " bytes");
        }
        // Common exit point - both branches reach here
    }

    public static function main() {
        var a = new ShortCircuitElse(-101, -102);
        trace("a.data = " + a.data);
        var b = new ShortCircuitElse(10, 20);
        trace("b.data = " + b.data);
    }
}
