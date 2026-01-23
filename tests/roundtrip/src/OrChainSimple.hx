// Simplified test: OR chain with non-terminating shared target

class OrChainSimple {
    var value:Int;

    public function new() {
        value = 0;
    }

    public function check(a:Bool, b:Bool, c:Bool):Int {
        // OR chain where body doesn't terminate
        if (a || b || c) {
            value = 100;
        }
        return value;
    }

    static function main() {
        var obj = new OrChainSimple();
        trace(obj.check(false, false, false)); // 0
        trace(obj.check(true, false, false));  // 100
        trace(obj.check(false, true, false));  // 100
    }
}
