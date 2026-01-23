// Test: OR chain with nested if in the body

class OrChainNested {
    var value:Int;
    var other:Int;

    public function new() {
        value = 0;
        other = 0;
    }

    public function check(a:Bool, b:Bool, reset:Bool):Int {
        // OR chain where body has nested control flow
        if (a || b) {
            if (reset) {
                other = 0;
            }
            value = 100;
        }
        return value;
    }

    static function main() {
        var obj = new OrChainNested();
        trace(obj.check(false, false, false)); // 0
        trace(obj.check(true, false, true));   // 100
    }
}
