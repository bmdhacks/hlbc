class NestedClass {
    public var data:Inner;

    public function new() {
        data = new Inner(42);
    }

    public function getValue():Int {
        return data.value;
    }

    public static function main() {
        var nc = new NestedClass();
        trace("Value: " + nc.getValue());
    }
}

private class Inner {
    public var value:Int;

    public function new(v:Int) {
        value = v;
    }
}
