// Tests static inline constant handling
// Bug: decompiler loses "inline" keyword and initial value
// Original: static inline var CONSTANT = 42;
// Decompiled: public static var CONSTANT: Int;

class StaticInline {
    static inline var ENABLED = true;
    static inline var MULTIPLIER = 10;
    static inline var OFFSET = 5.5;
    static inline var PREFIX = "test_";

    public static function compute(x:Int):Int {
        if (ENABLED) {
            return x * MULTIPLIER;
        }
        return x;
    }

    public static function adjusted(x:Float):Float {
        return x + OFFSET;
    }

    public static function named(s:String):String {
        return PREFIX + s;
    }

    public static function main() {
        Sys.println("compute(3)=" + compute(3));
        Sys.println("adjusted(1.5)=" + adjusted(1.5));
        Sys.println("named(foo)=" + named("foo"));

        // Direct use of inline constants
        Sys.println("ENABLED=" + ENABLED);
        Sys.println("MULTIPLIER=" + MULTIPLIER);
    }
}
