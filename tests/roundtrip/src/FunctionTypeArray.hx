class FunctionTypeArray {
    static function main() {
        // Array with function type elements
        var callbacks:Array<Int->Void> = [];
        callbacks.push(function(x:Int) { Sys.println("callback: " + x); });
        callbacks[0](42);

        // Array with complex function type
        var transforms:Array<Int->Int> = [];
        transforms.push(function(x:Int) { return x * 2; });
        Sys.println("result: " + transforms[0](5));

        // Nested function types
        var factories:Array<Int->(Int->Int)> = [];
        factories.push(function(n:Int) {
            return function(x:Int) { return x + n; };
        });
        var adder = factories[0](10);
        Sys.println("adder: " + adder(5));
    }
}
