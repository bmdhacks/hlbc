class StringSwitch {
    public static function main() {
        var result1 = describe("apple");
        var result2 = describe("banana");
        var result3 = describe("cherry");
        var result4 = describe("unknown");
        Sys.println('apple=' + result1);
        Sys.println('banana=' + result2);
        Sys.println('cherry=' + result3);
        Sys.println('unknown=' + result4);
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
