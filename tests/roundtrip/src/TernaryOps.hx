// Tests ternary operator decompilation
// Bug: decompiler expands ternary to verbose if/else blocks
// Original: var x = cond ? a : b;
// Decompiled: if (cond) { r4 = a; } else { r4 = b; } var x = r4;

class TernaryOps {
    static function sign(n:Int):Int {
        return n > 0 ? 1 : (n < 0 ? -1 : 0);
    }

    static function abs(n:Int):Int {
        return n >= 0 ? n : -n;
    }

    static function max(a:Int, b:Int):Int {
        return a > b ? a : b;
    }

    static function min(a:Int, b:Int):Int {
        return a < b ? a : b;
    }

    static function clamp(val:Int, lo:Int, hi:Int):Int {
        return val < lo ? lo : (val > hi ? hi : val);
    }

    static function describe(n:Int):String {
        return n == 0 ? "zero" : (n > 0 ? "positive" : "negative");
    }

    static function boolToInt(b:Bool):Int {
        return b ? 1 : 0;
    }

    static function main() {
        Sys.println("sign(-5)=" + sign(-5));
        Sys.println("sign(0)=" + sign(0));
        Sys.println("sign(3)=" + sign(3));

        Sys.println("abs(-7)=" + abs(-7));
        Sys.println("abs(4)=" + abs(4));

        Sys.println("max(3,8)=" + max(3, 8));
        Sys.println("min(3,8)=" + min(3, 8));

        Sys.println("clamp(5,0,10)=" + clamp(5, 0, 10));
        Sys.println("clamp(-3,0,10)=" + clamp(-3, 0, 10));
        Sys.println("clamp(15,0,10)=" + clamp(15, 0, 10));

        Sys.println("describe(-1)=" + describe(-1));
        Sys.println("boolToInt(true)=" + boolToInt(true));
    }
}
