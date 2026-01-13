class StringSwitch {
    public static function main() {
        var result1 = describe("apple");
        var result2 = describe("banana");
        var result3 = describe("cherry");
        var result4 = describe("unknown");
        trace('apple=' + result1);
        trace('banana=' + result2);
        trace('cherry=' + result3);
        trace('unknown=' + result4);
    }

    public static function describe(fruit:String):String {
        return switch (fruit) {
            case "apple": "red fruit";
            case "banana": "yellow fruit";
            case "cherry": "small red fruit";
            case "date": "brown fruit";
            case "elderberry": "purple berry";
            default: "unknown fruit";
        };
    }
}
