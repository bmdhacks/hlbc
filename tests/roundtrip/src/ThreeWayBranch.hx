class ThreeWayBranch {
    // Simple 3-way if/else-if/else with returns (no inlines)
    static function classify(a:Float, b:Float):String {
        if (a == b) {
            return "equal";
        } else if (a < b) {
            return "less";
        } else {
            return "greater";
        }
    }

    // 3-way with non-terminating branches and shared continuation
    static function adjust(a:Float, b:Float):Float {
        var result:Float;
        if (a == b) {
            result = 0.0;
        } else if (a < b) {
            result = b - a;
        } else {
            result = a - b;
        }
        return result + 1.0;
    }

    static function main() {
        Sys.println(classify(3.0, 3.0));
        Sys.println(classify(1.0, 5.0));
        Sys.println(classify(9.0, 2.0));
        Sys.println(adjust(3.0, 3.0));
        Sys.println(adjust(1.0, 5.0));
        Sys.println(adjust(9.0, 2.0));
    }
}
