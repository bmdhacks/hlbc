class IfElseIfChain {
    static function classify(x:Float, y:Float):String {
        if (x == y) {
            return "equal";
        } else if (x < y) {
            return "less";
        } else {
            return "greater";
        }
    }

    static function main() {
        Sys.println(classify(1.0, 2.0));
        Sys.println(classify(3.0, 3.0));
        Sys.println(classify(5.0, 4.0));
    }
}
