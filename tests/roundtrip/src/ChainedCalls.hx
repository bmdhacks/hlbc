// Test that chained method calls are properly inlined
// e.g., obj.getX().doY() should not create intermediate variables

class Inner {
    public var value:Int;

    public function new(v:Int) {
        this.value = v;
    }

    public function process(mult:Int):Int {
        return value * mult;
    }

    public function print():Void {
        trace("Inner value: " + value);
    }
}

class Outer {
    var inner:Inner;

    public function new(v:Int) {
        inner = new Inner(v);
    }

    public function getInner():Inner {
        return inner;
    }
}

class ChainedCalls {
    static function main() {
        var outer = new Outer(42);

        // This should decompile as: outer.getInner().print()
        // NOT as: var tmp = outer.getInner(); tmp.print();
        outer.getInner().print();

        // This should decompile as: trace(outer.getInner().process(2))
        // NOT as: var tmp = outer.getInner(); trace(tmp.process(2));
        var result = outer.getInner().process(2);
        trace("Result: " + result);
    }
}
