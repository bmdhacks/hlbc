class IfElseChain {
    static function main() {
        // Test if-else-if chain that should be detected as switch pattern
        var x = getInput();
        var result:String;

        if (x == 0) {
            result = "zero";
        } else if (x == 1) {
            result = "one";
        } else if (x == 2) {
            result = "two";
        } else if (x == 3) {
            result = "three";
        } else if (x == 4) {
            result = "four";
        } else {
            result = "other";
        }

        Sys.println("result=" + result);
    }

    // Prevent constant folding by using a function
    static function getInput():Int {
        return 2;
    }
}
