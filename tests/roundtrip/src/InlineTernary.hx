class InlineTernary {
    static inline function imin(a:Int, b:Int):Int {
        return a < b ? a : b;
    }

    static inline function imax(a:Int, b:Int):Int {
        return a > b ? a : b;
    }

    static inline function iclamp(v:Int, lo:Int, hi:Int):Int {
        return v < lo ? lo : (v > hi ? hi : v);
    }

    static inline function iabs(v:Int):Int {
        return v < 0 ? -v : v;
    }

    static function compute(x:Int, y:Int):Int {
        var a = imin(x, 100);
        var b = imax(y, 0);
        var c = iclamp(a + b, -50, 50);
        return iabs(c);
    }

    static function main() {
        Sys.println(compute(30, -20));
        Sys.println(compute(200, 5));
        Sys.println(compute(-10, -100));
    }
}
