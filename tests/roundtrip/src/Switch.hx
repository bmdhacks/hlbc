class Switch {
    static function main() {
        // Switch on integer
        var x = 2;
        var result = switch (x) {
            case 0: "zero";
            case 1: "one";
            case 2: "two";
            case 3: "three";
            default: "other";
        };
        trace("result=" + result);

        // Switch with combined cases
        var y = 5;
        var category = switch (y) {
            case 0, 1, 2: "small";
            case 3, 4, 5: "medium";
            default: "large";
        };
        trace("category=" + category);

        // Nested switch
        var a = 1;
        var b = 0;
        var nested = switch (a) {
            case 0: "a=0";
            case 1: switch (b) {
                case 0: "a=1,b=0";
                case 1: "a=1,b=1";
                default: "a=1,b=other";
            };
            default: "a=other";
        };
        trace("nested=" + nested);
    }
}
