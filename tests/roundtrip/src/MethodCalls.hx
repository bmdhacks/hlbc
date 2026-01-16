// Tests that method calls are decompiled with correct syntax
// Bug: decompiler outputs "methodName(obj, args)" instead of "obj.methodName(args)"
// Example: "setPriority(r16_2, 100)" instead of "r16_2.setPriority(100)"

class Helper {
    var name:String;
    var count:Int;

    public function new(n:String) {
        this.name = n;
        this.count = 0;
    }

    public function increment():Int {
        count++;
        return count;
    }

    public function add(amount:Int):Int {
        count += amount;
        return count;
    }

    public function format(prefix:String, suffix:String):String {
        return prefix + name + ":" + count + suffix;
    }

    public function reset():Void {
        count = 0;
    }
}

class MethodCalls {
    static function process(h:Helper):String {
        // Chain of method calls on an object
        h.reset();
        var a = h.increment();
        var b = h.add(5);
        var c = h.increment();
        return h.format("[", "]");
    }

    static function main() {
        var helper = new Helper("counter");

        // Direct method calls
        helper.increment();
        Sys.println("after inc: " + helper.format("(", ")"));

        helper.add(10);
        Sys.println("after add: " + helper.format("(", ")"));

        // Method calls through function
        var result = process(helper);
        Sys.println("processed: " + result);
    }
}
