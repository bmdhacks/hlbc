class TypedArrayInit {
    static function main() {
        // Various typed array initializations
        var ints:Array<Int> = [];
        var strings:Array<String> = [];
        var dynamics:Array<Dynamic> = [];

        ints.push(1);
        ints.push(2);
        ints.push(3);
        strings.push("hello");
        strings.push("world");
        dynamics.push({x: 1});
        dynamics.push({y: 2});

        Sys.println("ints: " + ints.length);
        Sys.println("strings: " + strings.length);
        Sys.println("dynamics: " + dynamics.length);

        // Typed array with objects
        var points:Array<{x:Int, y:Int}> = [];
        points.push({x: 10, y: 20});
        Sys.println("point: " + points[0].x + "," + points[0].y);

        // Array with nullable type
        var maybeInts:Array<Null<Int>> = [];
        maybeInts.push(5);
        maybeInts.push(null);
        Sys.println("maybeInts: " + maybeInts.length);
    }
}
