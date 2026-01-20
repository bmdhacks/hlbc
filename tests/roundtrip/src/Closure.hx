// Helper class that accepts callbacks
class Loader {
    public var value:Int;

    public function new(v:Int) {
        this.value = v;
    }

    public function loadAsync(callback:Int->Void):Void {
        callback(this.value);
    }
}

// Class with instance methods that pass `this`-bound callbacks
class Resource {
    public var data:Int;
    public var loader:Loader;

    public function new(loader:Loader) {
        this.data = 0;
        this.loader = loader;
    }

    // This should generate InstanceClosure bound to `this`
    public function load():Void {
        loader.loadAsync(function(v:Int):Void {
            this.data = v;  // captures `this`
            Sys.println("Loaded: " + this.data);
        });
    }

    // Method reference passed as callback (another InstanceClosure pattern)
    public function loadWithMethod():Void {
        loader.loadAsync(this.onLoaded);
    }

    public function onLoaded(v:Int):Void {
        this.data = v * 2;
        Sys.println("Method callback: " + this.data);
    }
}

class Closure {
    static function applyTwice(f:Int->Int, x:Int):Int {
        return f(f(x));
    }

    static function makeAdder(n:Int):Int->Int {
        return function(x:Int):Int {
            return x + n;
        };
    }

    static function makeMultiplier(n:Int):Int->Int {
        return function(x:Int):Int {
            return x * n;
        };
    }

    static function main() {
        // Inline lambda
        var double = function(x:Int):Int { return x * 2; };
        Sys.println("double(5)=" + double(5));
        Sys.println("doubled twice=" + applyTwice(double, 3));

        // Closure capturing variable
        var add5 = makeAdder(5);
        Sys.println("add5(10)=" + add5(10));

        var add10 = makeAdder(10);
        Sys.println("add10(7)=" + add10(7));

        // Another closure
        var mult3 = makeMultiplier(3);
        Sys.println("mult3(4)=" + mult3(4));

        // Instance closure bound to `this` (triggers InstanceClosure opcode)
        var loader = new Loader(42);
        var resource = new Resource(loader);
        resource.load();  // inline closure capturing `this`
        Sys.println("data after load: " + resource.data);

        // Method reference as callback (another InstanceClosure pattern)
        resource.loadWithMethod();
        Sys.println("data after loadWithMethod: " + resource.data);
    }
}
