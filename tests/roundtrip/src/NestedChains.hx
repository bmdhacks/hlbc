// Tests for nested AND/OR chains
// The Haxe compiler uses different jcond (jump condition) strategies:
// - OR: jump if condition is TRUE (to shared target)
// - AND: jump if condition is FALSE (to skip target), but last cond inherits outer jcond

class NestedChains {
    // AND inside OR: if (a < 0 || (b > 0 && c > 0) || d < 0) throw "invalid";
    // Bytecode structure:
    //   Block 0: if a < 0 -> throw (jcond=true)
    //   Block 2: if b <= 0 -> Block 6 (AND short-circuit to next OR condition)
    //   Block 4: if c > 0 -> throw (inherits outer jcond=true)
    //   Block 6: if d >= 0 -> continue (inverted), else -> throw
    public static function andInsideOr(a:Int, b:Int, c:Int, d:Int):Void {
        if (a < 0 || (b > 0 && c > 0) || d < 0)
            throw "invalid";
        Sys.println("valid");
    }

    // Multiple nested ANDs in OR: if (a < 0 || (b > 0 && c > 0) || (d > 0 && e > 0) || f < 0)
    public static function multipleNestedAnds(a:Int, b:Int, c:Int, d:Int, e:Int, f:Int):Void {
        if (a < 0 || (b > 0 && c > 0) || (d > 0 && e > 0) || f < 0)
            throw "invalid";
        Sys.println("valid");
    }

    // Simple test: just OR chain (baseline)
    public static function simpleOr(a:Int, b:Int, c:Int):Void {
        if (a < 0 || b < 0 || c < 0)
            throw "negative";
        Sys.println("all positive");
    }

    public static function main() {
        // Test andInsideOr
        try {
            andInsideOr(0, 0, 0, 0);
            Sys.println("andInsideOr(0,0,0,0) passed");
        } catch (e:Dynamic) {
            Sys.println("andInsideOr(0,0,0,0) threw: " + e);
        }

        try {
            andInsideOr(-1, 0, 0, 0);
        } catch (e:Dynamic) {
            Sys.println("andInsideOr(-1,0,0,0) correctly threw");
        }

        try {
            andInsideOr(0, 1, 1, 0);
        } catch (e:Dynamic) {
            Sys.println("andInsideOr(0,1,1,0) correctly threw (b>0 && c>0)");
        }

        try {
            andInsideOr(0, 0, 0, -1);
        } catch (e:Dynamic) {
            Sys.println("andInsideOr(0,0,0,-1) correctly threw (d<0)");
        }

        // Test multipleNestedAnds
        try {
            multipleNestedAnds(0, 0, 0, 0, 0, 0);
            Sys.println("multipleNestedAnds(0,0,0,0,0,0) passed");
        } catch (e:Dynamic) {
            Sys.println("multipleNestedAnds threw: " + e);
        }

        // Test simpleOr
        try {
            simpleOr(1, 1, 1);
            Sys.println("simpleOr(1,1,1) passed");
        } catch (e:Dynamic) {
            Sys.println("simpleOr threw: " + e);
        }
    }
}
